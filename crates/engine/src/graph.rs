//! Cœur temps réel du moteur : `AudioGraph::process` est appelé par le
//! callback cpal, et par les tests/benchs en rendu offline (mêmes structs,
//! specs §7). Tout ce module respecte les règles §2.2 : aucune allocation,
//! aucun lock, aucune I/O après construction.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::command::EngineCommand;
use crate::snapshot::EngineSnapshot;
use crate::track::TrackBuffer;
use crate::{CHANNELS, Deck};

const COMMAND_CAPACITY: usize = 1024;
const RECLAIM_CAPACITY: usize = 64;
/// ~0,25 s de stéréo 48 kHz. Le tap est best-effort : bloc entier ou rien,
/// jamais bloquant (specs §2.3).
const TAP_CAPACITY: usize = 24_000;

/// Extrémités non temps-réel des canaux du moteur (côté UI/workers).
pub struct EnginePorts {
    /// UI/MIDI → audio.
    pub commands: rtrb::Producer<EngineCommand>,
    /// Dernier état publié par le thread audio.
    pub snapshots: triple_buffer::Output<EngineSnapshot>,
    /// Buffers de piste renvoyés par le callback, à désallouer ici.
    pub reclaim: rtrb::Consumer<Arc<TrackBuffer>>,
    /// Samples post-mix pour les analyseurs temps réel (VU/FFT, M5).
    pub tap: rtrb::Consumer<f32>,
}

struct DeckState {
    track: Option<Arc<TrackBuffer>>,
    /// Position en frames. f64 : prêt pour la lecture fractionnaire du
    /// varispeed (M2) sans changer la structure.
    position: f64,
    playing: bool,
    volume: f32,
}

impl Default for DeckState {
    fn default() -> Self {
        Self {
            track: None,
            position: 0.0,
            playing: false,
            volume: 1.0,
        }
    }
}

pub struct AudioGraph {
    decks: [DeckState; 2],
    crossfader: f32,
    master_gain: f32,
    snapshot: EngineSnapshot,
    /// Callbacks ayant dépassé leur budget temps.
    budget_overruns: u64,
    commands: rtrb::Consumer<EngineCommand>,
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
        let (reclaim_tx, reclaim_rx) = rtrb::RingBuffer::new(RECLAIM_CAPACITY);
        let (tap_tx, tap_rx) = rtrb::RingBuffer::new(TAP_CAPACITY);
        let (snapshot_tx, snapshot_rx) =
            triple_buffer::TripleBuffer::new(&EngineSnapshot::default()).split();

        let graph = Self {
            decks: [DeckState::default(), DeckState::default()],
            crossfader: 0.0,
            master_gain: 1.0,
            snapshot: EngineSnapshot::default(),
            budget_overruns: 0,
            commands: commands_rx,
            reclaim: reclaim_tx,
            tap: tap_tx,
            snapshot_tx,
            stream_errors: Arc::new(AtomicU64::new(0)),
        };
        let ports = EnginePorts {
            commands: commands_tx,
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

    /// Remplit `out` (stéréo entrelacé) avec le mix des decks. Appelable
    /// depuis le callback cpal comme depuis un rendu offline.
    pub fn process(&mut self, out: &mut [f32]) {
        self.drain_commands();
        out.fill(0.0);

        let frames = out.len() / CHANNELS;
        let xf = crossfader_gains(self.crossfader);

        let decks = &mut self.decks;
        let deck_snapshots = &mut self.snapshot.decks;
        for ((deck, snap), xf_gain) in decks.iter_mut().zip(deck_snapshots.iter_mut()).zip(xf) {
            let mut sum_sq = [0.0f32; CHANNELS];
            let mut peak = [0.0f32; CHANNELS];

            if let Some(track) = deck.track.as_ref()
                && deck.playing
            {
                let gain = deck.volume * xf_gain;
                let total = track.frames();
                let mut pos = deck.position as usize;
                for frame in out.chunks_exact_mut(CHANNELS) {
                    if pos >= total {
                        deck.playing = false;
                        break;
                    }
                    let (l, r) = track.frame(pos);
                    let (sl, sr) = (l * gain, r * gain);
                    frame[0] += sl;
                    frame[1] += sr;
                    sum_sq[0] += sl * sl;
                    sum_sq[1] += sr * sr;
                    peak[0] = peak[0].max(sl.abs());
                    peak[1] = peak[1].max(sr.abs());
                    pos += 1;
                }
                deck.position = pos as f64;
            }

            let n = frames.max(1) as f32;
            snap.playing = deck.playing;
            snap.position_samples = deck.position as u64;
            snap.track_frames = deck.track.as_ref().map_or(0, |t| t.frames() as u64);
            snap.speed = if deck.playing { 1.0 } else { 0.0 };
            snap.rms = [(sum_sq[0] / n).sqrt(), (sum_sq[1] / n).sqrt()];
            snap.peak = peak;
        }

        // Gain master. Le limiteur soft-clip (obligatoire, specs §3.3)
        // arrive au M2 avec le reste du DSP.
        let mut sum_sq = [0.0f32; CHANNELS];
        let mut peak = [0.0f32; CHANNELS];
        for frame in out.chunks_exact_mut(CHANNELS) {
            frame[0] *= self.master_gain;
            frame[1] *= self.master_gain;
            sum_sq[0] += frame[0] * frame[0];
            sum_sq[1] += frame[1] * frame[1];
            peak[0] = peak[0].max(frame[0].abs());
            peak[1] = peak[1].max(frame[1].abs());
        }
        let n = frames.max(1) as f32;
        self.snapshot.master_rms = [(sum_sq[0] / n).sqrt(), (sum_sq[1] / n).sqrt()];
        self.snapshot.master_peak = peak;

        // Tap post-mix : bloc entier ou rien, pour préserver l'alignement
        // des canaux côté analyseurs.
        if self.tap.slots() >= out.len() {
            for &s in out.iter() {
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
        while let Ok(command) = self.commands.pop() {
            match command {
                EngineCommand::Play(d) => {
                    let deck = self.deck_mut(d);
                    deck.playing = deck.track.is_some();
                }
                EngineCommand::Pause(d) => self.deck_mut(d).playing = false,
                EngineCommand::SeekSamples(d, pos) => {
                    let deck = self.deck_mut(d);
                    let max = deck.track.as_ref().map_or(0, |t| t.frames() as u64);
                    deck.position = pos.min(max) as f64;
                }
                EngineCommand::SetDeckVolume(d, v) => {
                    self.deck_mut(d).volume = v.clamp(0.0, 1.0);
                }
                EngineCommand::SetCrossfader(x) => self.crossfader = x.clamp(-1.0, 1.0),
                EngineCommand::SetMasterGain(g) => self.master_gain = g.clamp(0.0, 2.0),
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
        let mut out = [0.0f32; 8];
        graph.process(&mut out);
        assert_eq!(graph.crossfader, 1.0);
        assert_eq!(graph.master_gain, 0.0);
    }
}
