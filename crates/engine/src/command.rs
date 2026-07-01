//! Commandes UI/MIDI → thread audio, transportées par un ring buffer SPSC
//! lock-free (`rtrb`, specs §2.3).
//!
//! Les variantes portent des valeurs prêtes à l'emploi — par exemple des
//! coefficients de biquad déjà calculés (M2), jamais des fréquences à
//! convertir : aucun travail différable n'entre dans le callback.

use std::sync::Arc;

use crate::Deck;
use crate::track::TrackBuffer;

/// S'étoffe à chaque jalon : coefficients d'EQ et routage cue (M2),
/// jog bend/scratch (M4).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EngineCommand {
    Play(Deck),
    Pause(Deck),
    /// Position absolue en samples (48 kHz), clampée à la fin de piste.
    SeekSamples(Deck, u64),
    /// Gain linéaire du deck, clampé à `[0, 1]` (courbe appliquée en amont).
    SetDeckVolume(Deck, f32),
    /// Position du crossfader `[-1, 1]` (A pleine gauche → B pleine droite).
    SetCrossfader(f32),
    /// Gain master linéaire, clampé à `[0, 2]`.
    SetMasterGain(f32),
    /// Remplace la piste du deck par échange de pointeur, sans copie
    /// (specs §3.4). L'ancien buffer repart par le canal de récupération.
    /// Le deck est remis à zéro (position 0, lecture arrêtée).
    SwapTrackBuffer(Deck, Arc<TrackBuffer>),
    /// Décharge la piste du deck.
    ClearTrack(Deck),
}
