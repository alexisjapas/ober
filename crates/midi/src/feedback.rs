//! Moteur de feedback générique (specs §5.2) : `état moteur → message MIDI`
//! d'après les bindings déclaratifs du mapping. Aucun code spécifique à un
//! contrôleur.
//!
//! Le moteur ne renvoie que les **changements** (diff par binding) : les
//! LEDs ne sont pas spammées à chaque tick, et un VU 7 bits n'émet que
//! quand sa valeur quantifiée bouge. `reset()` force un rafraîchissement
//! complet (reconnexion du contrôleur).

use engine::EngineSnapshot;
use mapping::{Deck, FeedbackBinding, FeedbackState, Mapping, Scale};

/// Seuil « fin de piste imminente » (specs §5.3, comportement Mixxx).
const END_OF_TRACK_SECONDS: f64 = 30.0;

pub struct FeedbackEngine {
    bindings: Vec<FeedbackBinding>,
    /// Dernière donnée émise par binding (None = jamais envoyée).
    last_sent: Vec<Option<u8>>,
    /// Rate of the opened output stream — snapshot positions are in samples
    /// at this rate (end-of-track threshold).
    sample_rate: u32,
}

impl FeedbackEngine {
    pub fn new(mapping: &Mapping, sample_rate: u32) -> Self {
        Self {
            last_sent: vec![None; mapping.feedback.len()],
            bindings: mapping.feedback.clone(),
            sample_rate,
        }
    }

    /// Force le renvoi de tous les états au prochain `refresh` (à appeler à
    /// la (re)connexion du contrôleur).
    pub fn reset(&mut self) {
        self.last_sent.fill(None);
    }

    /// Compare l'état courant aux derniers envois et pousse les messages
    /// MIDI 3 octets à émettre dans `out`.
    pub fn refresh(&mut self, snapshot: &EngineSnapshot, out: &mut Vec<[u8; 3]>) {
        for (binding, last) in self.bindings.iter().zip(self.last_sent.iter_mut()) {
            let value = state_value(binding.state, snapshot, self.sample_rate);
            let data = match binding.scale {
                Some(Scale::Linear(lo, hi)) => {
                    let (lo, hi) = (f32::from(lo), f32::from(hi));
                    (lo + value.clamp(0.0, 1.0) * (hi - lo)).round() as u8
                }
                None => {
                    if value >= 0.5 {
                        binding.on
                    } else {
                        binding.off
                    }
                }
            };
            if *last != Some(data) {
                let (status, data1) = binding.output.message_head();
                out.push([status, data1, data & 0x7F]);
                *last = Some(data);
            }
        }
    }
}

fn deck_index(deck: Deck) -> usize {
    match deck {
        Deck::A => 0,
        Deck::B => 1,
    }
}

/// Valeur 0..1 d'un état observable (binaire : 0.0 / 1.0).
fn state_value(state: FeedbackState, snapshot: &EngineSnapshot, sample_rate: u32) -> f32 {
    let deck = |d: Deck| &snapshot.decks[deck_index(d)];
    match state {
        FeedbackState::Playing { deck: d } => f32::from(u8::from(deck(d).playing)),
        FeedbackState::CueSet { deck: d } => f32::from(u8::from(deck(d).cue_point_samples > 0)),
        FeedbackState::HeadphoneCue { deck: d } => f32::from(u8::from(deck(d).cue)),
        FeedbackState::EndOfTrack { deck: d } => {
            let snap = deck(d);
            let near_end = snap.track_frames > 0
                && (snap.track_frames.saturating_sub(snap.position_samples) as f64)
                    < END_OF_TRACK_SECONDS * f64::from(sample_rate);
            f32::from(u8::from(near_end))
        }
        FeedbackState::VuMaster => (snapshot.master_rms[0] + snapshot.master_rms[1]) * 0.5,
        FeedbackState::VuDeck { deck: d } => {
            let snap = deck(d);
            (snap.rms[0] + snap.rms[1]) * 0.5
        }
        // Beatmatch guide : v0.2 (états réservés, éteints en v0.1).
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapping::FeedbackOutput;

    fn engine_with(bindings: Vec<FeedbackBinding>) -> FeedbackEngine {
        let mapping = Mapping {
            name: "test".into(),
            device_match: vec!["test".into()],
            init: vec![],
            controls: vec![],
            feedback: bindings,
            jog: mapping::JogConfig {
                ticks_per_rev: 720,
                touch_scratch: true,
                bend_sensitivity: 0.3,
                release_ramp_ms: 100,
                platter_rpm: 100.0 / 3.0,
                velocity_window_ms: 15.0,
                scratch_smoothing_ms: 5.0,
                bend_return_ms: 150.0,
            },
        };
        FeedbackEngine::new(&mapping, 48_000)
    }

    #[test]
    fn n_emet_que_les_changements() {
        let mut feedback = engine_with(vec![FeedbackBinding {
            state: FeedbackState::Playing { deck: Deck::A },
            output: FeedbackOutput::NoteOn { ch: 1, note: 0x07 },
            on: 0x7F,
            off: 0x00,
            scale: None,
        }]);
        let mut snapshot = EngineSnapshot::default();
        let mut out = Vec::new();

        // Premier refresh : état initial envoyé (LED éteinte).
        feedback.refresh(&snapshot, &mut out);
        assert_eq!(out, vec![[0x91, 0x07, 0x00]]);

        // Sans changement : silence.
        out.clear();
        feedback.refresh(&snapshot, &mut out);
        assert!(out.is_empty());

        // Lecture : LED allumée, une seule fois.
        snapshot.decks[0].playing = true;
        feedback.refresh(&snapshot, &mut out);
        feedback.refresh(&snapshot, &mut out);
        assert_eq!(out, vec![[0x91, 0x07, 0x7F]]);

        // Reset (reconnexion) : tout est renvoyé.
        out.clear();
        feedback.reset();
        feedback.refresh(&snapshot, &mut out);
        assert_eq!(out, vec![[0x91, 0x07, 0x7F]]);
    }

    #[test]
    fn vu_continu_avec_echelle() {
        let mut feedback = engine_with(vec![FeedbackBinding {
            state: FeedbackState::VuMaster,
            output: FeedbackOutput::CC { ch: 0, cc: 0x30 },
            on: 0x7F,
            off: 0x00,
            scale: Some(Scale::Linear(0, 127)),
        }]);
        let mut snapshot = EngineSnapshot::default();
        let mut out = Vec::new();

        snapshot.master_rms = [0.5, 0.5];
        feedback.refresh(&snapshot, &mut out);
        assert_eq!(out, vec![[0xB0, 0x30, 64]]);

        // Même valeur quantifiée : pas de renvoi.
        out.clear();
        snapshot.master_rms = [0.501, 0.501];
        feedback.refresh(&snapshot, &mut out);
        assert!(out.is_empty());

        snapshot.master_rms = [1.0, 1.0];
        feedback.refresh(&snapshot, &mut out);
        assert_eq!(out, vec![[0xB0, 0x30, 127]]);
    }

    #[test]
    fn etats_cue_et_fin_de_piste() {
        let mut feedback = engine_with(vec![
            FeedbackBinding {
                state: FeedbackState::CueSet { deck: Deck::A },
                output: FeedbackOutput::NoteOn { ch: 1, note: 0x06 },
                on: 0x7F,
                off: 0x00,
                scale: None,
            },
            FeedbackBinding {
                state: FeedbackState::EndOfTrack { deck: Deck::A },
                output: FeedbackOutput::NoteOn { ch: 1, note: 0x1C },
                on: 0x7F,
                off: 0x00,
                scale: None,
            },
        ]);
        let mut snapshot = EngineSnapshot::default();
        snapshot.decks[0].track_frames = 48_000 * 60; // 60 s
        snapshot.decks[0].cue_point_samples = 24_000;
        snapshot.decks[0].position_samples = 48_000 * 40; // reste 20 s < 30 s

        let mut out = Vec::new();
        feedback.refresh(&snapshot, &mut out);
        assert!(out.contains(&[0x91, 0x06, 0x7F]), "cue posé : {out:?}");
        assert!(out.contains(&[0x91, 0x1C, 0x7F]), "fin de piste : {out:?}");
    }
}
