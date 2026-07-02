//! Cœur temps réel du moteur : `AudioGraph::process` est appelé par le
//! callback cpal, et par les tests/benchs en rendu offline (mêmes structs,
//! specs §7). Tout ce module respecte les règles §2.2 : aucune allocation,
//! aucun lock, aucune I/O après construction.
//!
//! Chaîne par deck (specs §3.3) :
//!
//! ```text
//! piste → varispeed (Hermite 4 pts) → EQ 3 bandes → gain deck ─┬→ ×crossfader → Σ master → ×gain → soft-clip → out 1/2
//!                                        [tap cue si activé] ──┴→ Σ cue ──→ mix cue/master → ×gain casque → soft-clip → out 3/4
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::command::EngineCommand;
use crate::dsp::{StereoEq, hermite4, soft_clip};
use crate::jog::{JogRuntime, JogState};
use crate::snapshot::EngineSnapshot;
use crate::track::TrackBuffer;
use crate::{CHANNELS, Deck, MAX_BLOCK_FRAMES, MAX_PITCH_RATIO};

const COMMAND_CAPACITY: usize = 1024;
const RECLAIM_CAPACITY: usize = 64;
/// ~0,25 s de stéréo 48 kHz. Le tap est best-effort : bloc entier ou rien,
/// jamais bloquant (specs §2.3).
const TAP_CAPACITY: usize = 24_000;

/// Extrémités non temps-réel des canaux du moteur (côté UI/workers).
pub struct EnginePorts {
    /// UI → audio.
    pub commands: rtrb::Producer<EngineCommand>,
    /// Thread MIDI → audio : chemin court (specs §5.1), ring SPSC dédié —
    /// `take()` par l'app pour le donner au thread MIDI.
    pub midi_commands: Option<rtrb::Producer<EngineCommand>>,
    /// Dernier état publié par le thread audio.
    pub snapshots: triple_buffer::Output<EngineSnapshot>,
    /// Buffers de piste renvoyés par le callback, à désallouer ici.
    pub reclaim: rtrb::Consumer<Arc<TrackBuffer>>,
    /// Samples master post-limiteur pour le bus d'analyseurs temps réel.
    pub tap: rtrb::Consumer<f32>,
}

struct DeckState {
    track: Option<Arc<TrackBuffer>>,
    /// Position en frames, fractionnaire (lecture varispeed).
    position: f64,
    /// Vitesse de lecture (1.0 = nominale), clampée à ±16 %.
    speed: f64,
    /// Point cue en frames (sémantique vinyle, cf. `EngineCommand::CuePress`).
    cue_point: f64,
    /// Lecture temporaire tant que le bouton cue est tenu.
    previewing: bool,
    playing: bool,
    cue: bool,
    volume: f32,
    eq: StereoEq,
    jog: JogState,
    /// Vitesse effective de la dernière frame traitée (pour le snapshot et
    /// la continuité du scratch à la prise en main).
    last_speed: f64,
}

impl Default for DeckState {
    fn default() -> Self {
        Self {
            track: None,
            position: 0.0,
            speed: 1.0,
            cue_point: 0.0,
            previewing: false,
            playing: false,
            cue: false,
            volume: 1.0,
            eq: StereoEq::default(),
            jog: JogState::default(),
            last_speed: 0.0,
        }
    }
}

pub struct AudioGraph {
    decks: [DeckState; 2],
    jog_runtime: JogRuntime,
    crossfader: f32,
    master_gain: f32,
    /// Balance casque : 0.0 = cue seul, 1.0 = master seul.
    cue_mix: f32,
    headphone_gain: f32,
    /// 2 (master seul) ou 4 (master + casque), fixé avant le stream.
    output_channels: usize,
    /// Scratch pré-alloués (stéréo, MAX_BLOCK_FRAMES) — jamais réalloués.
    master_buf: Vec<f32>,
    cue_buf: Vec<f32>,
    snapshot: EngineSnapshot,
    /// Callbacks ayant dépassé leur budget temps.
    budget_overruns: u64,
    commands: rtrb::Consumer<EngineCommand>,
    midi_commands: rtrb::Consumer<EngineCommand>,
    reclaim: rtrb::Producer<Arc<TrackBuffer>>,
    tap: rtrb::Producer<f32>,
    snapshot_tx: triple_buffer::Input<EngineSnapshot>,
    /// Erreurs remontées par le callback d'erreur cpal (autre thread).
    stream_errors: Arc<AtomicU64>,
}

impl AudioGraph {
    #[allow(clippy::new_without_default)]
    pub fn new() -> (Self, EnginePorts) {
        let (commands_tx, commands_rx) = rtrb::RingBuffer::new(COMMAND_CAPACITY);
        let (midi_commands_tx, midi_commands_rx) = rtrb::RingBuffer::new(COMMAND_CAPACITY);
        let (reclaim_tx, reclaim_rx) = rtrb::RingBuffer::new(RECLAIM_CAPACITY);
        let (tap_tx, tap_rx) = rtrb::RingBuffer::new(TAP_CAPACITY);
        let (snapshot_tx, snapshot_rx) =
            triple_buffer::TripleBuffer::new(&EngineSnapshot::default()).split();

        let graph = Self {
            decks: [DeckState::default(), DeckState::default()],
            jog_runtime: JogRuntime::default(),
            crossfader: 0.0,
            master_gain: 1.0,
            cue_mix: 0.5,
            headphone_gain: 1.0,
            output_channels: CHANNELS,
            master_buf: vec![0.0; MAX_BLOCK_FRAMES * CHANNELS],
            cue_buf: vec![0.0; MAX_BLOCK_FRAMES * CHANNELS],
            snapshot: EngineSnapshot::default(),
            budget_overruns: 0,
            commands: commands_rx,
            midi_commands: midi_commands_rx,
            reclaim: reclaim_tx,
            tap: tap_tx,
            snapshot_tx,
            stream_errors: Arc::new(AtomicU64::new(0)),
        };
        let ports = EnginePorts {
            commands: commands_tx,
            midi_commands: Some(midi_commands_tx),
            snapshots: snapshot_rx,
            reclaim: reclaim_rx,
            tap: tap_rx,
        };
        (graph, ports)
    }

    /// Compteur partagé avec le callback d'erreur cpal (incrément atomique,
    /// lock-free).
    pub fn stream_error_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.stream_errors)
    }

    /// Nombre de canaux du stream de sortie : 2 (master seul) ou 4 (master
    /// 1/2 + casque 3/4). À fixer avant de démarrer le stream, jamais depuis
    /// le callback.
    pub fn set_output_channels(&mut self, channels: usize) {
        assert!(channels == 2 || channels == 4, "2 ou 4 canaux");
        self.output_channels = channels;
    }

    /// Remplit `out` (entrelacé, `output_channels` canaux) avec le mix.
    /// Appelable depuis le callback cpal comme depuis un rendu offline.
    pub fn process(&mut self, out: &mut [f32]) {
        self.drain_commands();
        out.fill(0.0);

        let channels = self.output_channels;
        let frames = (out.len() / channels).min(MAX_BLOCK_FRAMES);

        let master = &mut self.master_buf[..frames * 2];
        let cue = &mut self.cue_buf[..frames * 2];
        master.fill(0.0);
        cue.fill(0.0);

        let xf = crossfader_gains(self.crossfader);
        let jog_rt = self.jog_runtime;

        let decks = &mut self.decks;
        let deck_snapshots = &mut self.snapshot.decks;
        for ((deck, snap), xf_gain) in decks.iter_mut().zip(deck_snapshots.iter_mut()).zip(xf) {
            let mut sum_sq = [0.0f32; 2];
            let mut peak = [0.0f32; 2];
            let mut last_speed = 0.0f64;

            // Le jog peut imposer du son sur un deck à l'arrêt (scratch,
            // rampe de relâchement) — specs §3.5.
            if let Some(track) = deck.track.as_ref()
                && (deck.playing || deck.jog.engaged(&jog_rt))
            {
                let gain = deck.volume;
                let nominal = if deck.playing { deck.speed } else { 0.0 };
                for i in 0..frames {
                    let speed = deck.jog.effective_speed(nominal, &jog_rt);
                    if speed == 0.0 {
                        continue; // plateau tenu immobile : silence
                    }
                    let Some((l, r)) = varispeed_frame(track, &mut deck.position, speed) else {
                        deck.playing = false;
                        break;
                    };
                    last_speed = speed;
                    // EQ 3 bandes puis gain deck (chaîne §3.3).
                    let dl = deck.eq.process(0, l) * gain;
                    let dr = deck.eq.process(1, r) * gain;
                    // Tap cue : post-gain deck, pré-crossfader.
                    if deck.cue {
                        cue[i * 2] += dl;
                        cue[i * 2 + 1] += dr;
                    }
                    let (ml, mr) = (dl * xf_gain, dr * xf_gain);
                    master[i * 2] += ml;
                    master[i * 2 + 1] += mr;
                    sum_sq[0] += ml * ml;
                    sum_sq[1] += mr * mr;
                    peak[0] = peak[0].max(ml.abs());
                    peak[1] = peak[1].max(mr.abs());
                }
            }
            deck.last_speed = last_speed;

            let n = frames.max(1) as f32;
            snap.playing = deck.playing;
            snap.cue = deck.cue;
            snap.position_samples = deck.position as u64;
            snap.cue_point_samples = deck.cue_point as u64;
            snap.track_frames = deck.track.as_ref().map_or(0, |t| t.frames() as u64);
            snap.speed = deck.last_speed;
            snap.rms = [(sum_sq[0] / n).sqrt(), (sum_sq[1] / n).sqrt()];
            snap.peak = peak;
        }

        // Master : gain puis limiteur soft-clip (obligatoire, specs §3.3).
        let master_gain = self.master_gain;
        let mut sum_sq = [0.0f32; 2];
        let mut peak = [0.0f32; 2];
        for frame in master.chunks_exact_mut(2) {
            frame[0] = soft_clip(frame[0] * master_gain);
            frame[1] = soft_clip(frame[1] * master_gain);
            sum_sq[0] += frame[0] * frame[0];
            sum_sq[1] += frame[1] * frame[1];
            peak[0] = peak[0].max(frame[0].abs());
            peak[1] = peak[1].max(frame[1].abs());
        }
        let n = frames.max(1) as f32;
        self.snapshot.master_rms = [(sum_sq[0] / n).sqrt(), (sum_sq[1] / n).sqrt()];
        self.snapshot.master_peak = peak;

        // Écriture vers le périphérique.
        if channels == 4 {
            // out 1/2 = master ; out 3/4 = casque (cue mix, specs §3.3) :
            // hp = gain casque × ((1 − mix) × cue + mix × master), limité.
            let mix = self.cue_mix;
            let hp_gain = self.headphone_gain;
            for i in 0..frames {
                let (ml, mr) = (master[i * 2], master[i * 2 + 1]);
                let hl = soft_clip(hp_gain * ((1.0 - mix) * cue[i * 2] + mix * ml));
                let hr = soft_clip(hp_gain * ((1.0 - mix) * cue[i * 2 + 1] + mix * mr));
                let frame = &mut out[i * 4..i * 4 + 4];
                frame[0] = ml;
                frame[1] = mr;
                frame[2] = hl;
                frame[3] = hr;
            }
        } else {
            out[..frames * 2].copy_from_slice(master);
        }

        // Tap master post-limiteur : bloc entier ou rien, pour préserver
        // l'alignement des canaux côté analyseurs.
        if self.tap.slots() >= master.len() {
            for &s in master.iter() {
                let _ = self.tap.push(s);
            }
        }
    }

    /// À appeler après `process` avec le temps passé et le budget du bloc.
    pub fn record_callback(&mut self, busy: Duration, budget: Duration) {
        let load = if budget.is_zero() {
            0.0
        } else {
            (busy.as_secs_f64() / budget.as_secs_f64()) as f32
        };
        // Lissage exponentiel : lisible dans l'UI sans être nerveux.
        self.snapshot.callback_load = 0.9 * self.snapshot.callback_load + 0.1 * load;
        if load > 1.0 {
            self.budget_overruns += 1;
        }
    }

    /// Publie l'état courant vers l'UI (triple buffer, sans allocation).
    pub fn publish_snapshot(&mut self) {
        self.snapshot.underruns = self.budget_overruns + self.stream_errors.load(Ordering::Relaxed);
        self.snapshot_tx.write(self.snapshot);
    }

    fn drain_commands(&mut self) {
        // Deux rings SPSC : UI et thread MIDI (chemin court §5.1).
        while let Ok(command) = self.commands.pop() {
            self.apply_command(command);
        }
        while let Ok(command) = self.midi_commands.pop() {
            self.apply_command(command);
        }
    }

    fn apply_command(&mut self, command: EngineCommand) {
        {
            match command {
                EngineCommand::Play(d) => {
                    let deck = self.deck_mut(d);
                    deck.playing = deck.track.is_some();
                    deck.previewing = false;
                }
                EngineCommand::Pause(d) => {
                    let deck = self.deck_mut(d);
                    deck.playing = false;
                    deck.previewing = false;
                }
                EngineCommand::CuePress(d) => {
                    let deck = self.deck_mut(d);
                    if deck.track.is_none() {
                        return;
                    }
                    if deck.playing && !deck.previewing {
                        // En lecture : stop et retour au point cue.
                        deck.playing = false;
                        deck.position = deck.cue_point;
                    } else if !deck.playing {
                        if (deck.position - deck.cue_point).abs() < 1.0 {
                            // À l'arrêt sur le cue : pré-écoute tenue.
                            deck.previewing = true;
                            deck.playing = true;
                        } else {
                            // À l'arrêt ailleurs : pose le point cue ici.
                            deck.cue_point = deck.position;
                        }
                    }
                }
                EngineCommand::CueRelease(d) => {
                    let deck = self.deck_mut(d);
                    if deck.previewing {
                        deck.previewing = false;
                        deck.playing = false;
                        deck.position = deck.cue_point;
                    }
                }
                EngineCommand::SeekSamples(d, pos) => {
                    let deck = self.deck_mut(d);
                    let max = deck.track.as_ref().map_or(0, |t| t.frames() as u64);
                    deck.position = pos.min(max) as f64;
                }
                EngineCommand::SeekRelative(d, delta) => {
                    let deck = self.deck_mut(d);
                    let max = deck.track.as_ref().map_or(0.0, |t| t.frames() as f64);
                    deck.position = (deck.position + delta as f64).clamp(0.0, max);
                }
                EngineCommand::SetDeckVolume(d, v) => {
                    self.deck_mut(d).volume = v.clamp(0.0, 1.0);
                }
                EngineCommand::SetCrossfader(x) => self.crossfader = x.clamp(-1.0, 1.0),
                EngineCommand::SetMasterGain(g) => self.master_gain = g.clamp(0.0, 2.0),
                EngineCommand::SetPitch(d, speed) => {
                    self.deck_mut(d).speed =
                        speed.clamp(1.0 - MAX_PITCH_RATIO, 1.0 + MAX_PITCH_RATIO);
                }
                EngineCommand::SetEq(d, band, coeffs) => {
                    self.deck_mut(d).eq.set_band(band, coeffs);
                }
                EngineCommand::JogTouch(d, touched) => {
                    let jog_rt = self.jog_runtime;
                    let deck = self.deck_mut(d);
                    // Continuité : le freinage démarre de la vitesse réelle.
                    let current = if deck.last_speed != 0.0 {
                        deck.last_speed
                    } else if deck.playing {
                        deck.speed
                    } else {
                        0.0
                    };
                    deck.jog.set_touched(touched, current, &jog_rt);
                }
                EngineCommand::JogTicks(d, ticks) => self.deck_mut(d).jog.add_ticks(ticks),
                EngineCommand::SetJogParams(params) => self.jog_runtime = params.into(),
                EngineCommand::SetCueEnabled(d, on) => self.deck_mut(d).cue = on,
                EngineCommand::SetCueMix(x) => self.cue_mix = x.clamp(0.0, 1.0),
                EngineCommand::SetHeadphoneGain(g) => {
                    self.headphone_gain = g.clamp(0.0, 2.0);
                }
                EngineCommand::SwapTrackBuffer(d, track) => {
                    let deck = self.deck_mut(d);
                    let old = deck.track.replace(track);
                    deck.position = 0.0;
                    deck.playing = false;
                    self.send_to_reclaim(old);
                }
                EngineCommand::ClearTrack(d) => {
                    let deck = self.deck_mut(d);
                    let old = deck.track.take();
                    deck.position = 0.0;
                    deck.playing = false;
                    self.send_to_reclaim(old);
                }
            }
        }
    }

    fn deck_mut(&mut self, d: Deck) -> &mut DeckState {
        &mut self.decks[d.index()]
    }

    fn send_to_reclaim(&mut self, old: Option<Arc<TrackBuffer>>) {
        if let Some(old) = old {
            // Jamais de désallocation dans le callback (§2.2) : renvoi au
            // worker. Si le canal est plein, le drop de l'erreur ne fait que
            // décrémenter le compteur de l'Arc — l'UI conserve un clone de
            // chaque piste envoyée (cf. `track.rs`).
            let _ = self.reclaim.push(old);
        }
    }
}

/// Lecture d'une frame à position fractionnaire, interpolation Hermite
/// 4 points (specs §3.3/§3.5). Avance `position` de `speed` (négatif en
/// scratch arrière, clampé au début de piste). `None` en fin de piste.
/// Les voisins hors bornes sont clampés aux extrémités.
#[inline]
fn varispeed_frame(track: &TrackBuffer, position: &mut f64, speed: f64) -> Option<(f32, f32)> {
    let total = track.frames();
    let idx = *position as usize;
    if idx >= total {
        return None;
    }
    let t = (*position - idx as f64) as f32;
    let neighbor = |offset: isize| -> (f32, f32) {
        let j = (idx as isize + offset).clamp(0, total as isize - 1) as usize;
        track.frame(j)
    };
    let (lm1, rm1) = neighbor(-1);
    let (l0, r0) = neighbor(0);
    let (l1, r1) = neighbor(1);
    let (l2, r2) = neighbor(2);
    *position = (*position + speed).max(0.0);
    Some((hermite4(lm1, l0, l1, l2, t), hermite4(rm1, r0, r1, r2, t)))
}

/// Loi constant power (specs §3.3). `x` ∈ [-1, 1] → gains (deck A, deck B).
/// Les courbes configurables (sharp cut scratch…) arrivent avec le mapping.
fn crossfader_gains(x: f32) -> [f32; 2] {
    let t = (x.clamp(-1.0, 1.0) + 1.0) * 0.5;
    let angle = t * std::f32::consts::FRAC_PI_2;
    [angle.cos(), angle.sin()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfader_constant_power() {
        let [a, b] = crossfader_gains(-1.0);
        assert!((a - 1.0).abs() < 1e-6 && b.abs() < 1e-6);

        let [a, b] = crossfader_gains(1.0);
        assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6);

        // Au centre : -3 dB par deck, somme des puissances constante.
        let [a, b] = crossfader_gains(0.0);
        assert!((a - b).abs() < 1e-6);
        assert!((a * a + b * b - 1.0).abs() < 1e-5);
    }

    #[test]
    fn les_commandes_sont_clampees() {
        let (mut graph, mut ports) = AudioGraph::new();
        ports
            .commands
            .push(EngineCommand::SetCrossfader(42.0))
            .unwrap();
        ports
            .commands
            .push(EngineCommand::SetMasterGain(-3.0))
            .unwrap();
        ports
            .commands
            .push(EngineCommand::SetPitch(Deck::A, 3.0))
            .unwrap();
        let mut out = [0.0f32; 8];
        graph.process(&mut out);
        assert_eq!(graph.crossfader, 1.0);
        assert_eq!(graph.master_gain, 0.0);
        assert_eq!(graph.decks[0].speed, 1.0 + MAX_PITCH_RATIO);
    }

    #[test]
    fn varispeed_avance_a_la_vitesse_demandee() {
        let track = TrackBuffer::new(vec![0.0; 48_000 * 2]);
        let mut position = 0.0;
        for _ in 0..100 {
            varispeed_frame(&track, &mut position, 1.08);
        }
        assert!((position - 108.0).abs() < 1e-9);
    }
}
