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
//! | `B` (ou `F`/`L`)        | bibliothèque (explorateur intégré)  |
//! | `F12`                   | panneau préférences/diagnostics     |
//! | molette                 | zoom des waveforms                  |

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

mod browser;
mod fonts;
mod hud;
mod panel;
mod picker;
mod power;
mod theme;
mod vu;
mod waveform;
mod widgets;

use bevy::prelude::*;
use serde::Deserialize;

use engine::{Deck, Engine, EngineCommand, EngineConfig, EngineSnapshot, SAMPLE_RATE, TrackBuffer};
use midi::{ControlEvent, ControlValue, MidiIo, MidiStatus};

/// Résolution de l'overview waveform (specs §4.2 : ~1000 points/s).
const OVERVIEW_POINTS_PER_SECOND: u32 = 1_000;

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
const MAPPING_PATH: &str = "mappings/hercules_inpulse_200_mk2.ron";
/// Mapping par défaut embarqué (surchargé par le fichier s'il est présent).
const DEFAULT_MAPPING: &str = include_str!("../../../mappings/hercules_inpulse_200_mk2.ron");

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("ober {}", env!("CARGO_PKG_VERSION")),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .insert_resource(ClearColor(theme::color::BACKGROUND))
        .add_plugins((
            fonts::FontsPlugin,
            waveform::WaveformPlugin,
            vu::VuPlugin,
            hud::HudPlugin,
            widgets::WidgetsPlugin,
            panel::PanelPlugin,
            browser::BrowserPlugin,
            power::PowerPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (poll_decoded, midi_sync, keyboard_controls, drain_engine).chain(),
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

/// Messages des workers de chargement. La piste est jouable dès `Loaded` ;
/// l'analyse BPM/beatgrid arrive ensuite, asynchrone (specs §4.2).
enum WorkerMsg {
    Loaded {
        deck: Deck,
        name: String,
        truncated: bool,
        buffer: Arc<TrackBuffer>,
        summary: analysis::WaveformSummary,
    },
    LoadFailed {
        deck: Deck,
        name: String,
        error: decode::DecodeError,
    },
    Analyzed {
        deck: Deck,
        analysis: Option<analysis::TrackAnalysis>,
    },
}

#[derive(Resource)]
struct DecodeInbox(Mutex<Receiver<WorkerMsg>>);

struct LoadedTrack {
    name: String,
    /// Clone de l'Arc envoyé au moteur : garantit qu'un drop côté callback
    /// ne désalloue jamais (cf. engine::track) ; sert aussi au rendu.
    buffer: Arc<TrackBuffer>,
    /// Summary 3 bandes pour le rendu waveform (specs §4.2/§6.1).
    summary: analysis::WaveformSummary,
    /// BPM/beatgrid, livrés après coup par le worker (asynchrone).
    analysis: Option<analysis::TrackAnalysis>,
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
    /// Gains d'EQ en dB par deck et par bande (low/mid/high).
    eq_db: [[f32; 3]; 2],
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
            eq_db: [[0.0; 3]; 2],
            crossfader: 0.0,
            master: 1.0,
            cue_mix: 0.5,
            headphone: 1.0,
        }
    }
}

/// Émetteur vers les workers de chargement, conservé pour le file picker
/// (bouton Load, action MIDI, touches F/L).
#[derive(Resource)]
struct LoadSender(Sender<WorkerMsg>);

#[derive(Resource, Default)]
struct LastSnapshot(EngineSnapshot);

/// Sous-système MIDI : poignée du thread I/O + canaux de copie UI (§5.1).
#[derive(Resource)]
struct MidiRes {
    /// Maintient le thread MIDI en vie (drop = arrêt propre).
    _io: Option<MidiIo>,
    events: crossbeam_channel::Receiver<ControlEvent>,
    status: crossbeam_channel::Receiver<MidiStatus>,
    /// Relai des snapshots moteur vers le feedback LED (specs §5.2).
    snapshot_tx: crossbeam_channel::Sender<EngineSnapshot>,
    controller: Option<String>,
}

/// Bus d'analyseurs temps réel (specs §4.2), nourri par le tap audio.
/// v0.1 : niveaux RMS/peak pour les VU.
#[derive(Resource)]
struct Analyzers {
    bus: Mutex<analysis::AnalyzerBus>,
    block: Vec<f32>,
    frames: Vec<analysis::AnalysisFrame>,
    /// Derniers niveaux publiés (rms, peak) — consommés par la barre d'état
    /// (puis les VU-mètres du M6).
    levels: Option<([f32; 2], [f32; 2])>,
}

/// Charge le mapping contrôleur : fichier local prioritaire (itération sans
/// recompiler), sinon la copie embarquée. Valide avant usage (specs §5.2).
fn load_mapping() -> Option<mapping::Mapping> {
    let (source, text) = match std::fs::read_to_string(MAPPING_PATH) {
        Ok(text) => (MAPPING_PATH, text),
        Err(_) => ("mapping embarqué", DEFAULT_MAPPING.to_owned()),
    };
    let parsed: Result<mapping::Mapping, _> = text.parse();
    match parsed {
        Ok(mapping) => match mapping.validate() {
            Ok(()) => {
                info!("mapping « {} » chargé ({source})", mapping.name);
                Some(mapping)
            }
            Err(errors) => {
                for e in errors {
                    error!("mapping invalide : {e}");
                }
                None
            }
        },
        Err(e) => {
            error!("mapping illisible ({source}) : {e}");
            None
        }
    }
}

fn setup(mut commands: Commands) {
    let config = AppConfig::load();
    let engine_config = EngineConfig {
        device_match: config.device_match,
        buffer_frames: config.buffer_frames.unwrap_or(engine::TARGET_BUFFER_FRAMES),
    };
    let mut engine = Engine::start(engine_config).unwrap_or_else(|e| {
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
        spawn_load_worker(path, Deck::ALL[i], tx.clone());
    }

    // Thread MIDI : chemin court vers le moteur (ring SPSC dédié) + copie
    // des événements vers l'UI. Sans mapping valide, l'app reste utilisable
    // au clavier.
    let (midi_events_tx, midi_events_rx) = crossbeam_channel::unbounded();
    let (midi_status_tx, midi_status_rx) = crossbeam_channel::unbounded();
    let (snapshot_tx, snapshot_rx) = crossbeam_channel::unbounded();
    let midi_io = load_mapping().and_then(|mapping| {
        let producer = engine.ports.midi_commands.take()?;
        MidiIo::spawn(
            mapping,
            producer,
            midi_events_tx,
            midi_status_tx,
            snapshot_rx,
        )
        .inspect_err(|e| error!("thread MIDI : {e}"))
        .ok()
    });
    commands.insert_resource(MidiRes {
        _io: midi_io,
        events: midi_events_rx,
        status: midi_status_rx,
        snapshot_tx,
        controller: None,
    });

    let mut bus = analysis::AnalyzerBus::default();
    bus.register(Box::new(analysis::LevelsAnalyzer::new(2_048)));
    commands.insert_resource(Analyzers {
        bus: Mutex::new(bus),
        block: Vec::new(),
        frames: Vec::new(),
        levels: None,
    });

    commands.insert_resource(LoadSender(tx));
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
        match msg {
            WorkerMsg::Loaded {
                deck,
                name,
                truncated,
                buffer,
                summary,
            } => {
                if truncated {
                    warn!("« {name} » : fichier tronqué, partie décodée conservée");
                }
                info!(
                    "deck {:?} : « {} » chargée ({:.1} s)",
                    deck,
                    name,
                    buffer.duration_seconds()
                );
                decks.tracks[deck.index()] = Some(LoadedTrack {
                    name,
                    buffer: Arc::clone(&buffer),
                    summary,
                    analysis: None,
                });
                let mut eng = engine.0.lock().unwrap();
                if eng
                    .ports
                    .commands
                    .push(EngineCommand::SwapTrackBuffer(deck, buffer))
                    .is_err()
                {
                    warn!("file de commandes audio pleine, chargement ignoré");
                }
            }
            WorkerMsg::LoadFailed { deck, name, error } => {
                error!("deck {deck:?} : échec du décodage de « {name} » : {error}");
            }
            WorkerMsg::Analyzed { deck, analysis } => {
                if let Some(loaded) = decks.tracks[deck.index()].as_mut() {
                    match &analysis {
                        Some(a) => info!(
                            "deck {:?} : {:.2} BPM, premier beat à {:.2} s",
                            deck,
                            a.bpm,
                            a.first_beat_sample as f64 / f64::from(SAMPLE_RATE)
                        ),
                        None => info!("deck {deck:?} : tempo non détecté"),
                    }
                    loaded.analysis = analysis;
                }
            }
        }
    }
}

/// Worker de chargement : décode, calcule le summary 3 bandes, livre la
/// piste jouable puis l'analyse BPM/beatgrid en asynchrone (specs §4.2).
/// Utilisé par le chargement CLI et par le file picker.
fn spawn_load_worker(path: PathBuf, deck: Deck, tx: Sender<WorkerMsg>) {
    std::thread::Builder::new()
        .name(format!("decode-{deck:?}"))
        .spawn(move || {
            let name = path
                .file_name()
                .map_or_else(|| path.display().to_string(), |n| n.display().to_string());
            match decode::decode_file(&path) {
                Ok(track) => {
                    let truncated = track.truncated;
                    let summary = analysis::compute_summary(
                        &track.samples,
                        decode::TARGET_SAMPLE_RATE,
                        OVERVIEW_POINTS_PER_SECOND,
                    );
                    let buffer = TrackBuffer::new(track.samples);
                    // Jouable immédiatement…
                    let _ = tx.send(WorkerMsg::Loaded {
                        deck,
                        name,
                        truncated,
                        buffer: Arc::clone(&buffer),
                        summary,
                    });
                    // …le BPM/beatgrid arrive quand il est prêt (§4.2).
                    let analysis =
                        analysis::analyze_track(buffer.samples(), decode::TARGET_SAMPLE_RATE);
                    let _ = tx.send(WorkerMsg::Analyzed { deck, analysis });
                }
                Err(error) => {
                    let _ = tx.send(WorkerMsg::LoadFailed { deck, name, error });
                }
            }
        })
        .expect("spawn du thread de décodage");
}

/// Route un événement de contrôle vers le moteur (mêmes `mapping::Action`
/// que le MIDI, specs §6.4) et le reflète dans l'état d'affichage. Chemin
/// unique du clavier et des widgets souris.
fn emit_control(
    eng: &mut Engine,
    mix: &mut MixState,
    action: mapping::Action,
    value: ControlValue,
) {
    let event = ControlEvent { action, value };
    if let Some(command) = midi::to_engine_command(&event) {
        let _ = eng.ports.commands.push(command);
    }
    mirror_event(&event, mix);
}

/// Reflète un événement de contrôle dans l'état d'affichage — mêmes règles
/// pour le clavier, la souris et le MIDI (specs §6.4).
fn mirror_event(event: &ControlEvent, mix: &mut MixState) {
    use mapping::{Action as A, Deck as MDeck};
    use midi::ControlValue as V;
    let idx = |d: MDeck| match d {
        MDeck::A => 0usize,
        MDeck::B => 1,
    };
    match (event.action, event.value) {
        (A::Volume { deck }, V::Absolute(v)) => mix.volumes[idx(deck)] = v,
        (A::CrossFader, V::Absolute(v)) => mix.crossfader = v * 2.0 - 1.0,
        (A::Pitch { deck }, V::Absolute(v)) => {
            mix.pitch[idx(deck)] = (v * 2.0 - 1.0) * PITCH_RANGE;
        }
        (A::EqLow { deck }, V::Absolute(db)) => mix.eq_db[idx(deck)][0] = db,
        (A::EqMid { deck }, V::Absolute(db)) => mix.eq_db[idx(deck)][1] = db,
        (A::EqHigh { deck }, V::Absolute(db)) => mix.eq_db[idx(deck)][2] = db,
        (A::HeadphoneCue { deck }, V::Toggled(on) | V::Pressed(on)) => mix.cue[idx(deck)] = on,
        (A::MasterGain, V::Absolute(v)) => mix.master = v,
        (A::CueMix, V::Absolute(v)) => mix.cue_mix = v,
        (A::HeadphoneGain, V::Absolute(v)) => mix.headphone = v,
        _ => {}
    }
}

/// Copie UI du flux MIDI (specs §5.1) : le chemin court a déjà envoyé les
/// commandes au moteur depuis le thread MIDI ; ici on ne fait que refléter
/// les valeurs dans l'état d'affichage et traiter les actions purement UI.
fn midi_sync(
    mut midi: ResMut<MidiRes>,
    mut mix: ResMut<MixState>,
    mut browser: ResMut<browser::Browser>,
) {
    while let Ok(status) = midi.status.try_recv() {
        match status {
            MidiStatus::Connected(name) => {
                info!("contrôleur MIDI connecté : {name}");
                midi.controller = Some(name);
            }
            MidiStatus::Disconnected => {
                warn!("contrôleur MIDI débranché — reconnexion automatique en attente");
                midi.controller = None;
            }
        }
    }

    while let Ok(event) = midi.events.try_recv() {
        if let (mapping::Action::Load { .. }, ControlValue::Pressed(true)) =
            (event.action, event.value)
        {
            browser.open = true;
        }
        mirror_event(&event, &mut mix);
    }
}

/// Fallback clavier (specs §2.1) : émet les mêmes `mapping::Action` que le
/// MIDI — un seul chemin de traitement des intentions (specs §6.4), routé
/// vers le moteur par `midi::to_engine_command`.
fn keyboard_controls(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    engine: Res<AudioEngine>,
    mut mix: ResMut<MixState>,
    snapshot: Res<LastSnapshot>,
    mut browser: ResMut<browser::Browser>,
) {
    use mapping::{Action, Deck as MDeck};
    use midi::ControlValue as V;

    // Chargement de piste : explorateur intégré (touche B aussi).
    if keys.just_pressed(KeyCode::KeyF) || keys.just_pressed(KeyCode::KeyL) {
        browser.open = true;
    }

    let mut eng = engine.0.lock().unwrap();
    let mix = &mut *mix;
    let mut emit = |action: Action, value: V, mix: &mut MixState| {
        emit_control(&mut eng, mix, action, value);
    };

    const DECKS: [MDeck; 2] = [MDeck::A, MDeck::B];

    // Play/pause selon l'état réellement publié par le moteur.
    for (i, key) in [(0usize, KeyCode::Space), (1, KeyCode::Enter)] {
        if keys.just_pressed(key) {
            let playing = snapshot.0.decks[i].playing;
            emit(Action::Play { deck: DECKS[i] }, V::Toggled(!playing), mix);
        }
    }

    // Cue casque (toggle).
    for (i, key) in [(0usize, KeyCode::Digit1), (1, KeyCode::Digit2)] {
        if keys.just_pressed(key) {
            emit(
                Action::HeadphoneCue { deck: DECKS[i] },
                V::Toggled(!mix.cue[i]),
                mix,
            );
        }
    }

    // Seek ±5 s (le moteur clampe aux bornes de la piste).
    for (i, back, forward) in [
        (0usize, KeyCode::KeyA, KeyCode::KeyD),
        (1, KeyCode::ArrowLeft, KeyCode::ArrowRight),
    ] {
        let step = SEEK_STEP_SECONDS as i32;
        if keys.just_pressed(back) {
            emit(Action::Seek { deck: DECKS[i] }, V::Relative(-step), mix);
        }
        if keys.just_pressed(forward) {
            emit(Action::Seek { deck: DECKS[i] }, V::Relative(step), mix);
        }
    }

    // Contrôles continus (maintien de touche), paramétrés par le temps réel.
    let dt = time.delta_secs();
    let axis = |plus: KeyCode, minus: KeyCode| -> f32 {
        f32::from(keys.pressed(plus)) - f32::from(keys.pressed(minus))
    };

    for (i, plus, minus) in [
        (0usize, KeyCode::KeyW, KeyCode::KeyS),
        (1, KeyCode::ArrowUp, KeyCode::ArrowDown),
    ] {
        let delta = axis(plus, minus) * VOLUME_PER_SECOND * dt;
        if delta != 0.0 {
            let volume = (mix.volumes[i] + delta).clamp(0.0, 1.0);
            emit(Action::Volume { deck: DECKS[i] }, V::Absolute(volume), mix);
        }
    }

    // Pitch : même convention 0..1 que le fader MIDI (0,5 = nominal).
    for (i, plus, minus, reset_key) in [
        (0usize, KeyCode::KeyE, KeyCode::KeyQ, KeyCode::KeyR),
        (1, KeyCode::KeyO, KeyCode::KeyU, KeyCode::KeyP),
    ] {
        let delta = axis(plus, minus) * PITCH_PER_SECOND * dt;
        let reset = keys.just_pressed(reset_key);
        if delta != 0.0 || reset {
            let pitch = if reset {
                0.0
            } else {
                (mix.pitch[i] + delta).clamp(-PITCH_RANGE, PITCH_RANGE)
            };
            emit(
                Action::Pitch { deck: DECKS[i] },
                V::Absolute(pitch / PITCH_RANGE * 0.5 + 0.5),
                mix,
            );
        }
    }

    let xf_delta = axis(KeyCode::KeyV, KeyCode::KeyC) * CROSSFADER_PER_SECOND * dt;
    if xf_delta != 0.0 {
        let crossfader = (mix.crossfader + xf_delta).clamp(-1.0, 1.0);
        emit(Action::CrossFader, V::Absolute(crossfader * 0.5 + 0.5), mix);
    }

    let master_delta = axis(KeyCode::Equal, KeyCode::Minus) * MASTER_PER_SECOND * dt;
    if master_delta != 0.0 {
        let master = (mix.master + master_delta).clamp(0.0, 2.0);
        emit(Action::MasterGain, V::Absolute(master), mix);
    }

    let cue_mix_delta = axis(KeyCode::KeyM, KeyCode::KeyN) * CUE_MIX_PER_SECOND * dt;
    if cue_mix_delta != 0.0 {
        let cue_mix = (mix.cue_mix + cue_mix_delta).clamp(0.0, 1.0);
        emit(Action::CueMix, V::Absolute(cue_mix), mix);
    }

    let hp_delta = axis(KeyCode::KeyK, KeyCode::KeyJ) * HEADPHONE_PER_SECOND * dt;
    if hp_delta != 0.0 {
        let headphone = (mix.headphone + hp_delta).clamp(0.0, 2.0);
        emit(Action::HeadphoneGain, V::Absolute(headphone), mix);
    }
}

/// Draine chaque frame les canaux audio → UI : snapshot d'état (relayé au
/// feedback LED), récupération mémoire (les `Arc` se désallouent ici, côté
/// non temps réel) et tap audio → bus d'analyseurs (specs §4.2).
fn drain_engine(
    engine: Res<AudioEngine>,
    mut snapshot: ResMut<LastSnapshot>,
    midi: Res<MidiRes>,
    mut analyzers: ResMut<Analyzers>,
) {
    let mut eng = engine.0.lock().unwrap();
    snapshot.0 = *eng.ports.snapshots.read();
    // Copie vers le thread MIDI pour les LEDs (ignoré si thread absent).
    let _ = midi.snapshot_tx.send(snapshot.0);

    while eng.ports.reclaim.pop().is_ok() {}

    let analyzers = &mut *analyzers;
    analyzers.block.clear();
    while let Ok(sample) = eng.ports.tap.pop() {
        analyzers.block.push(sample);
    }
    if !analyzers.block.is_empty() {
        analyzers.frames.clear();
        analyzers
            .bus
            .lock()
            .unwrap()
            .process(&analyzers.block, &mut analyzers.frames);
        for frame in &analyzers.frames {
            if let analysis::AnalysisFrame::Levels { rms, peak } = frame {
                analyzers.levels = Some((*rms, *peak));
            }
        }
    }
}
