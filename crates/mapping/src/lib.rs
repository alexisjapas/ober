//! Format de mapping contrôleur déclaratif RON (specs §5.2).
//!
//! Un contrôleur = un fichier RON (cf. `mappings/`). Le moteur d'exécution
//! (crate `midi`) est générique : il traduit `événement MIDI → Action` et
//! `StateChange → message MIDI`. Aucun code spécifique à un contrôleur ici,
//! aucune dépendance Bevy.
//!
//! Note de syntaxe RON : `Absolute` s'écrit `Absolute()` (courbe linéaire
//! par défaut) ou `Absolute(curve: DbLinear(-26, 6))`.

use serde::{Deserialize, Serialize};

/// Identifiant de deck côté domaine/mapping. Converti vers `engine::Deck`
/// par les consommateurs (les deux crates restent indépendantes).
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
    Play {
        deck: Deck,
    },
    Cue {
        deck: Deck,
    },
    /// Déplacement dans la piste — `Relative` : secondes signées.
    Seek {
        deck: Deck,
    },
    Volume {
        deck: Deck,
    },
    EqLow {
        deck: Deck,
    },
    EqMid {
        deck: Deck,
    },
    EqHigh {
        deck: Deck,
    },
    Pitch {
        deck: Deck,
    },
    HeadphoneCue {
        deck: Deck,
    },
    /// Ticks relatifs du jog, surface touchée (scratch, M4).
    JogTick {
        deck: Deck,
    },
    /// Ticks relatifs du bord du jog, sans touch (pitch bend, M4).
    JogBend {
        deck: Deck,
    },
    /// Touch capacitif du jog.
    JogTouch {
        deck: Deck,
    },
    Load {
        deck: Deck,
    },
    /// Encodeur de navigation dans la bibliothèque (`Relative`) : la liste
    /// de fichiers (panneau de droite).
    LibraryScroll,
    /// Navigation dans le panneau des dossiers (`Relative`) — Shift +
    /// encodeur : la sélection d'un dossier prévisualise ses fichiers.
    LibraryFolderScroll,
    /// Poussoir de l'encodeur : entrer dans le dossier sélectionné.
    LibraryEnter,
    CrossFader,
    MasterGain,
    HeadphoneGain,
    CueMix,
    Shift,
}

/// Événement MIDI d'entrée à matcher (canaux 0–15, données 0–127).
/// Un binding `NoteOn` matche aussi le relâchement (NoteOff ou vélocité 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputSpec {
    NoteOn { ch: u8, note: u8 },
    NoteOff { ch: u8, note: u8 },
    CC { ch: u8, cc: u8 },
}

impl InputSpec {
    pub fn channel(&self) -> u8 {
        match *self {
            InputSpec::NoteOn { ch, .. }
            | InputSpec::NoteOff { ch, .. }
            | InputSpec::CC { ch, .. } => ch,
        }
    }

    pub fn data1(&self) -> u8 {
        match *self {
            InputSpec::NoteOn { note, .. } | InputSpec::NoteOff { note, .. } => note,
            InputSpec::CC { cc, .. } => cc,
        }
    }
}

/// Courbe appliquée à un contrôle absolu (0–127 normalisé en 0–1 d'abord).
/// La sortie est une valeur du domaine de l'action : gain linéaire pour
/// `Linear`, décibels pour `DbLinear` (consommés par ex. par l'EQ).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Curve {
    #[default]
    Linear,
    /// Linear with the direction flipped (`1 − t`) — hardware whose fader
    /// sends its maximum at the physical minimum (Pioneer tempo sliders).
    /// Flippable in the RON file without recompiling.
    InvertedLinear,
    /// Interpolation linéaire en dB entre (min, max).
    DbLinear(f32, f32),
}

impl Curve {
    /// `t` ∈ [0, 1] → valeur du domaine.
    pub fn apply(&self, t: f32) -> f32 {
        match *self {
            Curve::Linear => t,
            Curve::InvertedLinear => 1.0 - t,
            Curve::DbLinear(min, max) => min + t * (max - min),
        }
    }
}

/// Encodage des contrôles relatifs 7 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelativeEncoding {
    /// Signe-magnitude : bit 6 = signe. 0x01 = +1, 0x41 = −1.
    SignedBit,
    /// Complément à deux 7 bits : 0x01 = +1, 0x7F = −1 (jogs Hercules).
    TwosComplement,
    /// Décalage 64 (jogs et encodeurs Pioneer) : 0x41 = +1, 0x3F = −1,
    /// 0x40 = 0.
    Offset64,
}

impl RelativeEncoding {
    pub fn decode(&self, value: u8) -> i32 {
        let value = i32::from(value & 0x7F);
        match self {
            RelativeEncoding::SignedBit => {
                if value >= 0x40 {
                    -(value - 0x40)
                } else {
                    value
                }
            }
            RelativeEncoding::TwosComplement => {
                if value >= 0x40 {
                    value - 0x80
                } else {
                    value
                }
            }
            RelativeEncoding::Offset64 => value - 0x40,
        }
    }
}

/// Interprétation du contrôle (specs §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Mode {
    /// Chaque pression inverse un état tenu par le moteur de mapping.
    Toggle,
    /// Pression/relâchement transmis tels quels (boutons cue…).
    Momentary,
    /// Comme `Momentary`, pour les contrôles « tenus » (shift, jog touch).
    Gate,
    /// Contrôle continu 7 bits, courbe optionnelle.
    Absolute {
        #[serde(default)]
        curve: Curve,
    },
    /// Ticks relatifs signés (jogs, encodeurs).
    Relative { encoding: RelativeEncoding },
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
/// jamais en dur dans le code. Les champs optionnels ont des valeurs par
/// défaut raisonnables, ajustables à l'oreille contrôleur en main.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JogConfig {
    pub ticks_per_rev: u32,
    pub touch_scratch: bool,
    pub bend_sensitivity: f32,
    /// Rampe de retour à la vitesse nominale au relâchement (50–200 ms).
    pub release_ramp_ms: u32,
    /// Vitesse nominale du plateau virtuel (vinyle : 33⅓ tr/min).
    #[serde(default = "default_platter_rpm")]
    pub platter_rpm: f32,
    /// Fenêtre glissante d'estimation de vélocité (10–20 ms).
    #[serde(default = "default_velocity_window_ms")]
    pub velocity_window_ms: f32,
    /// Constante de temps de l'asservissement scratch (~5 ms).
    #[serde(default = "default_scratch_smoothing_ms")]
    pub scratch_smoothing_ms: f32,
    /// Constante de temps du retour progressif du bend.
    #[serde(default = "default_bend_return_ms")]
    pub bend_return_ms: f32,
}

fn default_platter_rpm() -> f32 {
    100.0 / 3.0 // 33⅓
}
fn default_velocity_window_ms() -> f32 {
    15.0
}
fn default_scratch_smoothing_ms() -> f32 {
    5.0
}
fn default_bend_return_ms() -> f32 {
    150.0
}

/// État observable pour le feedback LED (specs §5.2/§5.3). Les états du
/// beatmatch guide (v0.2) sont réservés dès maintenant.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum FeedbackState {
    Playing {
        deck: Deck,
    },
    /// Un point cue est posé sur le deck.
    CueSet {
        deck: Deck,
    },
    HeadphoneCue {
        deck: Deck,
    },
    /// Fin de piste imminente (< 30 s restantes).
    EndOfTrack {
        deck: Deck,
    },
    /// Niveaux continus (à associer à `scale`).
    VuMaster,
    VuDeck {
        deck: Deck,
    },
    // --- beatmatch guide, v0.2 (réservés, valent 0 en v0.1) ---
    BeatmatchTempoFaster {
        deck: Deck,
    },
    BeatmatchTempoSlower {
        deck: Deck,
    },
    BeatmatchPhaseAhead {
        deck: Deck,
    },
    BeatmatchPhaseBehind {
        deck: Deck,
    },
}

/// Message MIDI sortant d'un feedback.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FeedbackOutput {
    NoteOn { ch: u8, note: u8 },
    CC { ch: u8, cc: u8 },
}

impl FeedbackOutput {
    pub fn channel(&self) -> u8 {
        match *self {
            FeedbackOutput::NoteOn { ch, .. } | FeedbackOutput::CC { ch, .. } => ch,
        }
    }

    /// (statut, data1) du message MIDI.
    pub fn message_head(&self) -> (u8, u8) {
        match *self {
            FeedbackOutput::NoteOn { ch, note } => (0x90 | (ch & 0x0F), note),
            FeedbackOutput::CC { ch, cc } => (0xB0 | (ch & 0x0F), cc),
        }
    }
}

/// Mise à l'échelle d'un état continu (VU) vers la donnée MIDI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Scale {
    Linear(u8, u8),
}

fn default_on() -> u8 {
    0x7F
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackBinding {
    pub state: FeedbackState,
    pub output: FeedbackOutput,
    /// Valeur émise quand l'état binaire est actif.
    #[serde(default = "default_on")]
    pub on: u8,
    /// Valeur émise quand l'état binaire est inactif.
    #[serde(default)]
    pub off: u8,
    /// Pour les états continus (VU) : mapping 0..1 → plage MIDI.
    #[serde(default)]
    pub scale: Option<Scale>,
}

/// Mapping complet d'un contrôleur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mapping {
    pub name: String,
    /// Substrings matchés sur le nom du port MIDI (détection automatique).
    pub device_match: Vec<String>,
    /// Messages MIDI bruts envoyés à la connexion (ex. activation du mode
    /// « full MIDI » des LEDs Hercules). Le trait `ControllerBackend`
    /// prendra le relais si un contrôleur exige plus (SysEx…).
    #[serde(default)]
    pub init: Vec<(u8, u8, u8)>,
    #[serde(default)]
    pub controls: Vec<ControlBinding>,
    /// Feedback LED/VU : `StateChange → message MIDI` (specs §5.2).
    #[serde(default)]
    pub feedback: Vec<FeedbackBinding>,
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
    /// Vrai si `port_name` correspond à ce contrôleur.
    pub fn matches_port(&self, port_name: &str) -> bool {
        self.device_match.iter().any(|m| port_name.contains(m))
    }

    /// Validation sémantique, erreurs lisibles (specs §5.2). Retourne la
    /// liste complète des problèmes plutôt que le premier rencontré.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.device_match.is_empty() {
            errors.push("device_match vide : le contrôleur ne sera jamais détecté".into());
        }
        if self.jog.ticks_per_rev == 0 {
            errors.push("jog.ticks_per_rev doit être > 0".into());
        }

        for (i, control) in self.controls.iter().enumerate() {
            let ctx = format!("controls[{i}] ({:?})", control.action);
            if control.input.channel() > 15 {
                errors.push(format!(
                    "{ctx} : canal {} hors plage 0–15",
                    control.input.channel()
                ));
            }
            if control.input.data1() > 127 {
                errors.push(format!(
                    "{ctx} : note/cc {} hors plage 0–127",
                    control.input.data1()
                ));
            }
            if let Mode::Absolute {
                curve: Curve::DbLinear(min, max),
            } = control.mode
                && min >= max
            {
                errors.push(format!(
                    "{ctx} : courbe DbLinear({min}, {max}) invalide (min ≥ max)"
                ));
            }

            for (j, other) in self.controls.iter().enumerate().skip(i + 1) {
                if control.input == other.input && control.shift == other.shift {
                    errors.push(format!(
                        "contrôle dupliqué : controls[{i}] et controls[{j}] partagent {:?} (shift: {})",
                        control.input, control.shift
                    ));
                }
            }
        }

        for (i, feedback) in self.feedback.iter().enumerate() {
            let ctx = format!("feedback[{i}] ({:?})", feedback.state);
            if feedback.output.channel() > 15 {
                errors.push(format!(
                    "{ctx} : canal {} hors plage 0–15",
                    feedback.output.channel()
                ));
            }
            if feedback.on > 127 || feedback.off > 127 {
                errors.push(format!("{ctx} : valeurs on/off hors plage 0–127"));
            }
            if let Some(Scale::Linear(lo, hi)) = feedback.scale
                && (lo > 127 || hi > 127)
            {
                errors.push(format!("{ctx} : échelle hors plage 0–127"));
            }
            for (j, other) in self.feedback.iter().enumerate().skip(i + 1) {
                if feedback.output == other.output {
                    errors.push(format!(
                        "feedback dupliqué : feedback[{i}] et feedback[{j}] partagent {:?}",
                        feedback.output
                    ));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERCULES: &str = include_str!("../../../mappings/hercules_inpulse_200_mk2.ron");
    const DDJ_400: &str = include_str!("../../../mappings/pioneer_ddj_400.ron");

    #[test]
    fn le_mapping_hercules_livre_est_parseable_et_valide() {
        let m: Mapping = HERCULES.parse().expect("mapping RON invalide");
        assert_eq!(m.name, "Hercules DJControl Inpulse 200 MK2");
        assert!(m.matches_port("DJControl Inpulse 200 MK2 MIDI 1"));
        assert!(!m.controls.is_empty());
        assert!(!m.init.is_empty(), "message d'init LEDs attendu");
        if let Err(errors) = m.validate() {
            panic!("mapping invalide : {errors:#?}");
        }
    }

    #[test]
    fn shipped_ddj_400_mapping_parses_and_validates() {
        let m: Mapping = DDJ_400.parse().expect("mapping RON invalide");
        assert_eq!(m.name, "Pioneer DDJ-400");
        assert!(m.matches_port("DDJ-400 MIDI 1"));
        assert!(!m.controls.is_empty());
        // Software shift layer: the rotary is bound on both layers.
        assert!(m.controls.iter().any(|c| c.shift));
        if let Err(errors) = m.validate() {
            panic!("mapping invalide : {errors:#?}");
        }
    }

    #[test]
    fn les_doublons_et_hors_plage_sont_detectes() {
        let mut m: Mapping = HERCULES.parse().unwrap();
        let first = m.controls[0].clone();
        m.controls.push(first);
        m.controls.push(ControlBinding {
            input: InputSpec::CC { ch: 42, cc: 200 },
            shift: false,
            action: Action::CrossFader,
            mode: Mode::Absolute {
                curve: Curve::default(),
            },
        });
        let errors = m.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("dupliqué")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("canal 42")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("200")), "{errors:?}");
    }

    #[test]
    fn courbes_et_encodages() {
        assert_eq!(Curve::Linear.apply(0.5), 0.5);
        let db = Curve::DbLinear(-26.0, 6.0);
        assert_eq!(db.apply(0.0), -26.0);
        assert_eq!(db.apply(1.0), 6.0);

        let tc = RelativeEncoding::TwosComplement;
        assert_eq!(tc.decode(0x01), 1);
        assert_eq!(tc.decode(0x7F), -1);
        assert_eq!(tc.decode(0x02), 2);
        assert_eq!(tc.decode(0x7E), -2);

        let sb = RelativeEncoding::SignedBit;
        assert_eq!(sb.decode(0x01), 1);
        assert_eq!(sb.decode(0x41), -1);
        assert_eq!(sb.decode(0x43), -3);

        let off = RelativeEncoding::Offset64;
        assert_eq!(off.decode(0x40), 0);
        assert_eq!(off.decode(0x41), 1);
        assert_eq!(off.decode(0x3F), -1);
        assert_eq!(off.decode(0x45), 5);

        assert_eq!(Curve::InvertedLinear.apply(0.0), 1.0);
        assert_eq!(Curve::InvertedLinear.apply(1.0), 0.0);
    }
}
