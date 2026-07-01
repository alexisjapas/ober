//! Commandes UI/MIDI → thread audio, transportées par un ring buffer SPSC
//! lock-free (`rtrb`, specs §2.3).
//!
//! Les variantes portent des valeurs prêtes à l'emploi — par exemple des
//! coefficients de biquad déjà calculés (M2), jamais des fréquences à
//! convertir : aucun travail différable n'entre dans le callback.

use crate::Deck;

/// Squelette M0 — s'étoffe à chaque jalon : `SwapTrackBuffer` (M1),
/// coefficients d'EQ et routage cue (M2), jog bend/scratch (M4).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum EngineCommand {
    Play(Deck),
    Pause(Deck),
    /// Position absolue en samples (48 kHz).
    SeekSamples(Deck, u64),
    /// Gain linéaire du deck, `[0, 1]` (courbe appliquée en amont).
    SetDeckVolume(Deck, f32),
    /// Position du crossfader `[-1, 1]` (A pleine gauche → B pleine droite).
    SetCrossfader(f32),
    /// Gain master linéaire.
    SetMasterGain(f32),
}
