//! Binaire Bevy : UI, orchestration, plugins (specs §6). Seule crate du
//! workspace autorisée à dépendre de Bevy (frontière §1.4, vérifiée en CI).
//!
//! M1/M2 : mix 2 pistes au clavier, EQ/varispeed/limiteur, pré-écoute casque
//! si la carte du contrôleur est détectée — pas encore de rendu (waveforms et
//! design system : M6). Usage :
//!
//! ```sh
//! cargo run -p app -- piste_a.mp3 piste_b.flac
//! ```
//!
//! Configuration optionnelle dans `ober.config.ron` (répertoire courant) :
//! périphérique audio (`device_match`) et taille de buffer.
//!
//! Contrôles (positions physiques, étiquettes QWERTY) :
//!
//! | Touche                  | Action                              |
//! |-------------------------|-------------------------------------|
//! | `Espace` / `Entrée`     | play/pause deck A / deck B          |
//! | `A` `D`                 | seek deck A −5 s / +5 s             |
//! | `←` `→`                 | seek deck B −5 s / +5 s             |
//! | `W` `S`                 | volume deck A + / −                 |
//! | `↑` `↓`                 | volume deck B + / −                 |
//! | `C` `V`                 | crossfader vers A / vers B          |
//! | `-` `=`                 | gain master − / +                   |
//! | `1` / `2`               | cue casque deck A / deck B          |
//! | `Q` `E`                 | pitch deck A − / + (±8 %)           |
//! | `U` `O`                 | pitch deck B − / + (±8 %)           |
//! | `R` / `P`               | remise à zéro du pitch A / B        |
//! | `N` `M`                 | mix casque cue ↔ master             |
//! | `J` `K`                 | gain casque − / +                   |

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use serde::Deserialize;

use engine::{Deck, Engine, EngineCommand, EngineConfig, EngineSnapshot, SAMPLE_RATE, TrackBuffer};

const SEEK_STEP_SECONDS: u64 = 5;
const VOLUME_PER_SECOND: f32 = 0.8;
const CROSSFADER_PER_SECOND: f32 = 1.5;
const MASTER_PER_SECOND: f32 = 0.8;
/// Plage pitch clavier : ±8 % (le ±16 % complet arrive avec le fader MIDI).
const PITCH_RANGE: f32 = 0.08;
const PITCH_PER_SECOND: f32 = 0.04;
const CUE_MIX_PER_SECOND: f32 = 0.8;
const HEADPHONE_PER_SECOND: f32 = 0.8;

const CONFIG_PATH: &str = "ober.config.ron";

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("ober {}", env!("CARGO_PKG_VERSION")),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (poll_decoded, keyboard_controls, drain_engine, update_status).chain(),
        )
        .run();
}

/// Configuration optionnelle (`ober.config.ron`, specs §3.2).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AppConfig {
    /// Substring du nom du périphérique de sortie. Absent → détection
    /// automatique "DJControl" puis périphérique par défaut.
    device_match: Option<String>,
    buffer_frames: Option<u32>,
}

impl AppConfig {
    fn load() -> Self {
        match std::fs::read_to_string(CONFIG_PATH) {
            Ok(text) => match ron::from_str(&text) {
                Ok(config) => {
                    info!("configuration lue depuis {CONFIG_PATH}");
                    config
                }
                Err(e) => {
                    error!("{CONFIG_PATH} invalide ({e}) — configuration par défaut");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }
}

/// Le moteur n'est pas `Sync` (producteurs SPSC) : accès sérialisé par un
/// Mutex, pris brièvement par les systèmes du thread principal uniquement —
/// jamais sur le chemin audio.
#[derive(Resource)]
struct AudioEngine(Mutex<Engine>);

struct DecodedMsg {
    deck: Deck,
    name: String,
    result: Result<decode::DecodedTrack, decode::DecodeError>,
}

#[derive(Resource)]
struct DecodeInbox(Mutex<Receiver<DecodedMsg>>);

struct LoadedTrack {
    name: String,
    /// Clone de l'Arc envoyé au moteur, jamais lu au M1/M2 (le rendu
    /// waveform M6 s'en servira) : garantit qu'un drop côté callback ne
    /// désalloue jamais (cf. engine::track).
    _buffer: Arc<TrackBuffer>,
}

#[derive(Resource, Default)]
struct Decks {
    tracks: [Option<LoadedTrack>; 2],
}

/// Valeurs pilotées par l'UI (le moteur reste la source de vérité pour la
/// position/lecture, publiée par snapshots).
#[derive(Resource)]
struct MixState {
    volumes: [f32; 2],
    /// Fraction de pitch (−0,08 → +0,08) ; le moteur reçoit 1.0 + pitch.
    pitch: [f32; 2],
    cue: [bool; 2],
    crossfader: f32,
    master: f32,
    /// 0.0 = cue seul, 1.0 = master seul.
    cue_mix: f32,
    headphone: f32,
}

impl Default for MixState {
    fn default() -> Self {
        Self {
            volumes: [1.0, 1.0],
            pitch: [0.0, 0.0],
            cue: [false, false],
            crossfader: 0.0,
            master: 1.0,
            cue_mix: 0.5,
            headphone: 1.0,
        }
    }
}

#[derive(Resource, Default)]
struct LastSnapshot(EngineSnapshot);

fn setup(mut commands: Commands) {
    let config = AppConfig::load();
    let engine_config = EngineConfig {
        device_match: config.device_match,
        buffer_frames: config.buffer_frames.unwrap_or(engine::TARGET_BUFFER_FRAMES),
    };
    let engine = Engine::start(engine_config).unwrap_or_else(|e| {
        panic!("impossible de démarrer le moteur audio : {e}");
    });
    info!(
        "audio : « {} » @ {} Hz, {} canaux ({}), buffer {} (latence buffer ≈ {})",
        engine.info.device_name,
        engine.info.sample_rate,
        engine.info.channels,
        if engine.info.headphone_active() {
            "master + casque"
        } else {
            "master seul, pas de pré-écoute"
        },
        engine
            .info
            .buffer_frames
            .map_or("par défaut".to_owned(), |f| format!("{f} frames")),
        engine
            .info
            .buffer_latency_ms()
            .map_or("?".to_owned(), |ms| format!("{ms:.1} ms")),
    );

    // Chargement des pistes passées en CLI (file picker natif : M6).
    let (tx, rx) = channel();
    let paths: Vec<PathBuf> = std::env::args()
        .skip(1)
        .take(2)
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        info!("aucune piste en argument — usage : ober <piste_a> [piste_b]");
    }
    for (i, path) in paths.into_iter().enumerate() {
        let deck = Deck::ALL[i];
        let tx = tx.clone();
        std::thread::Builder::new()
            .name(format!("decode-{deck:?}"))
            .spawn(move || {
                let name = path
                    .file_name()
                    .map_or_else(|| path.display().to_string(), |n| n.display().to_string());
                let result = decode::decode_file(&path);
                let _ = tx.send(DecodedMsg { deck, name, result });
            })
            .expect("spawn du thread de décodage");
    }

    commands.insert_resource(AudioEngine(Mutex::new(engine)));
    commands.insert_resource(DecodeInbox(Mutex::new(rx)));
    commands.insert_resource(Decks::default());
    commands.insert_resource(MixState::default());
    commands.insert_resource(LastSnapshot::default());

    info!(
        "contrôles : Espace/Entrée play·pause — A/D ←/→ seek — W/S ↑/↓ volumes — C/V crossfader \
         — -/= master — 1/2 cue — Q/E U/O pitch (R/P reset) — N/M mix casque — J/K gain casque"
    );
}

/// Récupère les pistes décodées par les workers et les installe dans le
/// moteur par échange de pointeur (specs §3.4).
fn poll_decoded(inbox: Res<DecodeInbox>, engine: Res<AudioEngine>, mut decks: ResMut<Decks>) {
    let rx = inbox.0.lock().unwrap();
    while let Ok(msg) = rx.try_recv() {
        match msg.result {
            Ok(track) => {
                if track.truncated {
                    warn!(
                        "« {} » : fichier tronqué, partie décodée conservée",
                        msg.name
                    );
                }
                let buffer = TrackBuffer::new(track.samples);
                info!(
                    "deck {:?} : « {} » chargée ({:.1} s)",
                    msg.deck,
                    msg.name,
                    buffer.duration_seconds()
                );
                decks.tracks[msg.deck.index()] = Some(LoadedTrack {
                    name: msg.name,
                    _buffer: Arc::clone(&buffer),
                });
                let mut eng = engine.0.lock().unwrap();
                if eng
                    .ports
                    .commands
                    .push(EngineCommand::SwapTrackBuffer(msg.deck, buffer))
                    .is_err()
                {
                    warn!("file de commandes audio pleine, chargement ignoré");
                }
            }
            Err(e) => error!(
                "deck {:?} : échec du décodage de « {} » : {e}",
                msg.deck, msg.name
            ),
        }
    }
}

/// Fallback clavier (specs §2.1) : émet les mêmes commandes moteur que le
/// futur chemin MIDI. À partir du M3, clavier et MIDI passeront par les
/// mêmes `mapping::Action` (specs §6.4).
fn keyboard_controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    engine: Res<AudioEngine>,
    mut mix: ResMut<MixState>,
    snapshot: Res<LastSnapshot>,
) {
    let mut eng = engine.0.lock().unwrap();

    // Play/pause selon l'état réellement publié par le moteur.
    for (deck, key) in [(Deck::A, KeyCode::Space), (Deck::B, KeyCode::Enter)] {
        if keys.just_pressed(key) {
            let playing = snapshot.0.decks[deck.index()].playing;
            let command = if playing {
                EngineCommand::Pause(deck)
            } else {
                EngineCommand::Play(deck)
            };
            let _ = eng.ports.commands.push(command);
        }
    }

    // Cue casque (toggle).
    for (deck, key) in [(Deck::A, KeyCode::Digit1), (Deck::B, KeyCode::Digit2)] {
        if keys.just_pressed(key) {
            let cue = &mut mix.cue[deck.index()];
            *cue = !*cue;
            let _ = eng
                .ports
                .commands
                .push(EngineCommand::SetCueEnabled(deck, *cue));
        }
    }

    // Seek ±5 s depuis la position publiée.
    let seek_step = SEEK_STEP_SECONDS * u64::from(SAMPLE_RATE);
    for (deck, back, forward) in [
        (Deck::A, KeyCode::KeyA, KeyCode::KeyD),
        (Deck::B, KeyCode::ArrowLeft, KeyCode::ArrowRight),
    ] {
        let position = snapshot.0.decks[deck.index()].position_samples;
        if keys.just_pressed(back) {
            let _ = eng.ports.commands.push(EngineCommand::SeekSamples(
                deck,
                position.saturating_sub(seek_step),
            ));
        }
        if keys.just_pressed(forward) {
            let _ = eng
                .ports
                .commands
                .push(EngineCommand::SeekSamples(deck, position + seek_step));
        }
    }

    // Contrôles continus (maintien de touche), paramétrés par le temps réel.
    let dt = time.delta_secs();
    let axis = |plus: KeyCode, minus: KeyCode| -> f32 {
        f32::from(keys.pressed(plus)) - f32::from(keys.pressed(minus))
    };

    for (deck, plus, minus) in [
        (Deck::A, KeyCode::KeyW, KeyCode::KeyS),
        (Deck::B, KeyCode::ArrowUp, KeyCode::ArrowDown),
    ] {
        let delta = axis(plus, minus) * VOLUME_PER_SECOND * dt;
        if delta != 0.0 {
            let volume = &mut mix.volumes[deck.index()];
            *volume = (*volume + delta).clamp(0.0, 1.0);
            let _ = eng
                .ports
                .commands
                .push(EngineCommand::SetDeckVolume(deck, *volume));
        }
    }

    // Pitch : maintien pour glisser, R/P pour revenir à zéro.
    for (deck, plus, minus, reset_key) in [
        (Deck::A, KeyCode::KeyE, KeyCode::KeyQ, KeyCode::KeyR),
        (Deck::B, KeyCode::KeyO, KeyCode::KeyU, KeyCode::KeyP),
    ] {
        let delta = axis(plus, minus) * PITCH_PER_SECOND * dt;
        let reset = keys.just_pressed(reset_key);
        if delta != 0.0 || reset {
            let pitch = &mut mix.pitch[deck.index()];
            *pitch = if reset {
                0.0
            } else {
                (*pitch + delta).clamp(-PITCH_RANGE, PITCH_RANGE)
            };
            let _ = eng
                .ports
                .commands
                .push(EngineCommand::SetPitch(deck, f64::from(1.0 + *pitch)));
        }
    }

    let xf_delta = axis(KeyCode::KeyV, KeyCode::KeyC) * CROSSFADER_PER_SECOND * dt;
    if xf_delta != 0.0 {
        mix.crossfader = (mix.crossfader + xf_delta).clamp(-1.0, 1.0);
        let _ = eng
            .ports
            .commands
            .push(EngineCommand::SetCrossfader(mix.crossfader));
    }

    let master_delta = axis(KeyCode::Equal, KeyCode::Minus) * MASTER_PER_SECOND * dt;
    if master_delta != 0.0 {
        mix.master = (mix.master + master_delta).clamp(0.0, 2.0);
        let _ = eng
            .ports
            .commands
            .push(EngineCommand::SetMasterGain(mix.master));
    }

    let cue_mix_delta = axis(KeyCode::KeyM, KeyCode::KeyN) * CUE_MIX_PER_SECOND * dt;
    if cue_mix_delta != 0.0 {
        mix.cue_mix = (mix.cue_mix + cue_mix_delta).clamp(0.0, 1.0);
        let _ = eng
            .ports
            .commands
            .push(EngineCommand::SetCueMix(mix.cue_mix));
    }

    let hp_delta = axis(KeyCode::KeyK, KeyCode::KeyJ) * HEADPHONE_PER_SECOND * dt;
    if hp_delta != 0.0 {
        mix.headphone = (mix.headphone + hp_delta).clamp(0.0, 2.0);
        let _ = eng
            .ports
            .commands
            .push(EngineCommand::SetHeadphoneGain(mix.headphone));
    }
}

/// Draine chaque frame les canaux audio → UI : snapshot d'état, récupération
/// mémoire (les `Arc` se désallouent ici, côté non temps réel) et tap audio
/// (ignoré pour l'instant — le bus d'analyseurs arrive au M5).
fn drain_engine(engine: Res<AudioEngine>, mut snapshot: ResMut<LastSnapshot>) {
    let mut eng = engine.0.lock().unwrap();
    snapshot.0 = *eng.ports.snapshots.read();
    while eng.ports.reclaim.pop().is_ok() {}
    while eng.ports.tap.pop().is_ok() {}
}

/// Barre d'état minimale : tout dans le titre de fenêtre, rafraîchi à ~4 Hz.
/// La vraie barre d'état (specs §6.3) arrive au M6.
fn update_status(
    time: Res<Time>,
    snapshot: Res<LastSnapshot>,
    decks: Res<Decks>,
    mix: Res<MixState>,
    mut windows: Query<&mut Window>,
    mut accumulator: Local<f32>,
) {
    *accumulator += time.delta_secs();
    if *accumulator < 0.25 {
        return;
    }
    *accumulator = 0.0;

    let deck_status = |i: usize| -> String {
        let snap = &snapshot.0.decks[i];
        let name = decks.tracks[i].as_ref().map_or("—", |t| t.name.as_str());
        let state = if snap.playing { "▶" } else { "⏸" };
        let cue = if snap.cue { " CUE" } else { "" };
        format!(
            "{state} {} {}/{} {:+.1}%{cue}",
            name,
            format_time(snap.position_samples),
            format_time(snap.track_frames),
            mix.pitch[i] * 100.0,
        )
    };

    let title = format!(
        "ober — A {} | B {} | xf {:+.2} | master {:.2} | casque {:.2} mix {:.2} | underruns {} | charge audio {:.0} %",
        deck_status(0),
        deck_status(1),
        mix.crossfader,
        mix.master,
        mix.headphone,
        mix.cue_mix,
        snapshot.0.underruns,
        snapshot.0.callback_load * 100.0
    );
    for mut window in &mut windows {
        window.title = title.clone();
    }
}

fn format_time(samples: u64) -> String {
    let seconds = samples / u64::from(SAMPLE_RATE);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
