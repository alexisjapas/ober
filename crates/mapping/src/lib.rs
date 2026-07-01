//! Format de mapping contrôleur déclaratif RON (specs §5.2).
//!
//! Un contrôleur = un fichier RON (cf. `mappings/`). Le moteur d'exécution
//! (crate `midi`) est générique : il traduit `événement MIDI → Action` et
//! `StateChange → message MIDI`. Aucun code spécifique à un contrôleur ici,
//! aucune dépendance Bevy.

use serde::{Deserialize, Serialize};

/// Identifiant de deck côté domaine/mapping. Converti vers `engine::Deck`
/// par la crate `app` (les deux crates restent indépendantes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Deck {
    A,
    B,
}

/// Vocabulaire des intentions du domaine. Les interactions UI émettent les
/// mêmes actions que le MIDI : un seul chemin de traitement (specs §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Action {
    Play { deck: Deck },
    Cue { deck: Deck },
    Volume { deck: Deck },
    EqLow { deck: Deck },
    EqMid { deck: Deck },
    EqHigh { deck: Deck },
    Pitch { deck: Deck },
    HeadphoneCue { deck: Deck },
    JogTick { deck: Deck },
    JogTouch { deck: Deck },
    Load { deck: Deck },
    CrossFader,
    MasterGain,
    HeadphoneGain,
    CueMix,
    Shift,
}

/// Événement MIDI d'entrée à matcher (canaux 0–15, données 0–127).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputSpec {
    NoteOn { ch: u8, note: u8 },
    NoteOff { ch: u8, note: u8 },
    CC { ch: u8, cc: u8 },
}

/// Modes de base (M0). Le jalon M3 ajoute les courbes (`DbLinear`…) pour
/// `Absolute` et les encodages (`SignedBit`…) pour `Relative`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Mode {
    Toggle,
    Momentary,
    Gate,
    Absolute,
    Relative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlBinding {
    pub input: InputSpec,
    /// Vrai si le binding n'est actif que couche Shift enfoncée.
    #[serde(default)]
    pub shift: bool,
    pub action: Action,
    pub mode: Mode,
}

/// Paramètres du modèle de jog (specs §3.5) — vivent dans le mapping,
/// jamais en dur dans le code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JogConfig {
    pub ticks_per_rev: u32,
    pub touch_scratch: bool,
    pub bend_sensitivity: f32,
    pub release_ramp_ms: u32,
}

/// Mapping complet d'un contrôleur. Le schéma `feedback` (LEDs, VU) est
/// conçu au jalon M5 — réserver les états dès maintenant côté specs §5.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mapping {
    pub name: String,
    /// Substrings matchés sur le nom du port MIDI (détection automatique).
    pub device_match: Vec<String>,
    #[serde(default)]
    pub controls: Vec<ControlBinding>,
    pub jog: JogConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum MappingError {
    #[error("RON invalide : {0}")]
    Parse(String),
}

impl std::str::FromStr for Mapping {
    type Err = MappingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ron::from_str(s).map_err(|e| MappingError::Parse(e.to_string()))
    }
}

impl Mapping {
    /// Validation sémantique — erreurs lisibles : contrôle dupliqué, canal
    /// hors plage… (specs §5.2). Implémentation : jalon M3.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERCULES: &str = include_str!("../../../mappings/hercules_inpulse_200_mk2.ron");

    #[test]
    fn le_mapping_hercules_livre_est_parseable() {
        let m: Mapping = HERCULES.parse().expect("mapping RON invalide");
        assert_eq!(m.name, "Hercules DJControl Inpulse 200 MK2");
        assert!(m.device_match.iter().any(|s| s.contains("DJControl")));
        assert!(m.validate().is_ok());
    }
}
