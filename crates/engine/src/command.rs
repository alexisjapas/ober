//! Commandes UI/MIDI → thread audio, transportées par un ring buffer SPSC
//! lock-free (`rtrb`, specs §2.3).
//!
//! Les variantes portent des valeurs prêtes à l'emploi — par exemple des
//! coefficients de biquad déjà calculés, jamais des fréquences à
//! convertir : aucun travail différable n'entre dans le callback.

use std::sync::Arc;

use crate::Deck;
use crate::dsp::{BiquadCoeffs, EqBand};
use crate::jog::JogRuntime;
use crate::track::TrackBuffer;

/// S'étoffe à chaque jalon : jog bend/scratch au M4.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EngineCommand {
    Play(Deck),
    Pause(Deck),
    /// Bouton cue pressé — sémantique vinyle résolue par le moteur, qui
    /// connaît son état : en lecture → stop et retour au point cue ; à
    /// l'arrêt ailleurs qu'au cue → pose le point cue ; à l'arrêt sur le
    /// cue → pré-écoute tant que le bouton est tenu.
    CuePress(Deck),
    /// Relâchement du bouton cue : fin de pré-écoute, retour au point cue.
    CueRelease(Deck),
    /// Position absolue en samples (48 kHz), clampée à la fin de piste.
    SeekSamples(Deck, u64),
    /// Déplacement relatif en samples signés, clampé aux bornes de la piste.
    SeekRelative(Deck, i64),
    /// Gain linéaire du deck, clampé à `[0, 1]` (courbe appliquée en amont).
    SetDeckVolume(Deck, f32),
    /// Position du crossfader `[-1, 1]` (A pleine gauche → B pleine droite).
    SetCrossfader(f32),
    /// Gain master linéaire, clampé à `[0, 2]`.
    SetMasterGain(f32),
    /// Vitesse de lecture (1.0 = nominale), clampée à ±16 % (specs §1.2).
    /// Comportement vinyle : le pitch modifie la hauteur (specs §3.3).
    SetPitch(Deck, f64),
    /// Coefficients d'une bande d'EQ, calculés hors callback via
    /// `dsp::eq_coeffs` (specs §3.3 : les commandes portent les
    /// coefficients, jamais les fréquences).
    SetEq(Deck, EqBand, BiquadCoeffs),
    /// Active/désactive la pré-écoute casque du deck (specs §3.3).
    SetCueEnabled(Deck, bool),
    /// Balance cue/master du casque : 0.0 = cue seul, 1.0 = master seul.
    SetCueMix(f32),
    /// Gain casque linéaire, clampé à `[0, 2]`.
    SetHeadphoneGain(f32),
    /// Touch capacitif du jog pressé/relâché (specs §3.5).
    JogTouch(Deck, bool),
    /// Ticks relatifs de rotation du jog (crans signés). Le mode — scratch
    /// (surface touchée) ou bend (bord) — est déterminé par l'état du touch
    /// côté moteur.
    JogTicks(Deck, i32),
    /// Jog model coefficients, derived outside the callback from the RON
    /// mapping's [`JogParams`] and the stream's sample rate via
    /// [`JogRuntime::new`] (specs §3.5: nothing hard-coded; Rule 5: commands
    /// carry ready-to-use values). Applied to both decks.
    SetJogParams(JogRuntime),
    /// Remplace la piste du deck par échange de pointeur, sans copie
    /// (specs §3.4). L'ancien buffer repart par le canal de récupération.
    /// Le deck est remis à zéro (position 0, lecture arrêtée).
    SwapTrackBuffer(Deck, Arc<TrackBuffer>),
    /// Décharge la piste du deck.
    ClearTrack(Deck),
}

/// Paramètres du modèle scratch/bend (specs §3.5), unités SI. Convertis en
/// coefficients par bloc/sample côté moteur, hors callback critique.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JogParams {
    pub ticks_per_rev: f64,
    /// Faux : le touch est ignoré, toute rotation est un bend.
    pub touch_scratch: bool,
    pub bend_sensitivity: f64,
    /// Rampe de retour à la vitesse nominale au relâchement (s).
    pub release_ramp: f64,
    /// Tours/seconde nominaux du plateau virtuel (33⅓ tr/min ≈ 0,556).
    pub platter_rev_per_s: f64,
    /// Fenêtre glissante d'estimation de vélocité (s).
    pub velocity_window: f64,
    /// Constante de temps de l'asservissement scratch (s).
    pub scratch_smoothing: f64,
    /// Constante de temps du retour progressif du bend (s).
    pub bend_return: f64,
}

impl Default for JogParams {
    fn default() -> Self {
        Self {
            ticks_per_rev: 720.0,
            touch_scratch: true,
            bend_sensitivity: 0.3,
            release_ramp: 0.1,
            platter_rev_per_s: 100.0 / 180.0, // 33⅓ tr/min
            velocity_window: 0.015,
            scratch_smoothing: 0.005,
            bend_return: 0.15,
        }
    }
}
