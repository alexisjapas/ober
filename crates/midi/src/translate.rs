//! Moteur de mapping générique (specs §5.2) : traduit un message MIDI brut
//! en événement de contrôle du domaine, d'après un `mapping::Mapping`
//! déclaratif. AUCUN code spécifique à un contrôleur ici.
//!
//! Le moteur tient l'état de la couche Shift et des toggles ; les courbes
//! des contrôles absolus sont appliquées ici (la valeur émise est dans le
//! domaine de l'action : gain 0–1 pour `Linear`, dB pour `DbLinear`).

use mapping::{Action, ControlBinding, InputSpec, Mapping, Mode};

/// Valeur portée par un événement de contrôle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlValue {
    /// Bouton momentané/gate : pressé (true) ou relâché (false).
    Pressed(bool),
    /// Toggle résolu par le moteur de mapping : nouvel état.
    Toggled(bool),
    /// Contrôle absolu, courbe déjà appliquée.
    Absolute(f32),
    /// Ticks relatifs signés (jogs, encodeurs).
    Relative(i32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlEvent {
    pub action: Action,
    pub value: ControlValue,
}

/// Message MIDI canal décodé (voix uniquement — SysEx et temps réel ignorés).
#[derive(Debug, Clone, Copy)]
enum ChannelMessage {
    NoteOn { ch: u8, note: u8 },
    NoteOff { ch: u8, note: u8 },
    ControlChange { ch: u8, cc: u8, value: u8 },
}

fn parse(bytes: &[u8]) -> Option<ChannelMessage> {
    let (&status, data) = bytes.split_first()?;
    let ch = status & 0x0F;
    match status & 0xF0 {
        0x90 => {
            let (&note, rest) = data.split_first()?;
            let velocity = rest.first().copied().unwrap_or(0);
            if velocity == 0 {
                // NoteOn vélocité 0 ≡ NoteOff (usage MIDI courant).
                Some(ChannelMessage::NoteOff { ch, note })
            } else {
                Some(ChannelMessage::NoteOn { ch, note })
            }
        }
        0x80 => {
            let &note = data.first()?;
            Some(ChannelMessage::NoteOff { ch, note })
        }
        0xB0 => {
            let (&cc, rest) = data.split_first()?;
            let value = rest.first().copied().unwrap_or(0);
            Some(ChannelMessage::ControlChange { ch, cc, value })
        }
        _ => None,
    }
}

pub struct MappingEngine {
    controls: Vec<ControlBinding>,
    /// État des toggles, parallèle à `controls`.
    toggles: Vec<bool>,
    shift: bool,
}

impl MappingEngine {
    pub fn new(mapping: &Mapping) -> Self {
        Self {
            toggles: vec![false; mapping.controls.len()],
            controls: mapping.controls.clone(),
            shift: false,
        }
    }

    pub fn shift_active(&self) -> bool {
        self.shift
    }

    /// Traduit un message brut. `None` si le message ne matche aucun binding
    /// (ou n'a pas d'effet, ex. relâchement d'un toggle).
    pub fn translate(&mut self, bytes: &[u8]) -> Option<ControlEvent> {
        let message = parse(bytes)?;

        // (clé de matching, pression/valeur)
        let (key, payload) = match message {
            ChannelMessage::NoteOn { ch, note } => {
                (InputSpec::NoteOn { ch, note }, Payload::Press(true))
            }
            ChannelMessage::NoteOff { ch, note } => {
                // Un NoteOff matche le binding NoteOn correspondant.
                (InputSpec::NoteOn { ch, note }, Payload::Press(false))
            }
            ChannelMessage::ControlChange { ch, cc, value } => {
                (InputSpec::CC { ch, cc }, Payload::Value(value))
            }
        };

        let index = self.find_binding(key)?;
        let binding = &self.controls[index];
        let action = binding.action;

        let value = match (binding.mode, payload) {
            (Mode::Toggle, Payload::Press(true)) => {
                self.toggles[index] = !self.toggles[index];
                ControlValue::Toggled(self.toggles[index])
            }
            (Mode::Toggle, Payload::Press(false)) => return None,
            (Mode::Momentary | Mode::Gate, Payload::Press(pressed)) => {
                if action == Action::Shift {
                    self.shift = pressed;
                }
                ControlValue::Pressed(pressed)
            }
            (Mode::Absolute { curve }, Payload::Value(v)) => {
                let t = f32::from(v & 0x7F) / 127.0;
                ControlValue::Absolute(curve.apply(t))
            }
            (Mode::Relative { encoding }, Payload::Value(v)) => {
                ControlValue::Relative(encoding.decode(v))
            }
            // Combinaisons incohérentes (ex. bouton mappé en Absolute) :
            // interprétation de secours plutôt que panique.
            (Mode::Absolute { curve }, Payload::Press(pressed)) => {
                ControlValue::Absolute(curve.apply(if pressed { 1.0 } else { 0.0 }))
            }
            (Mode::Toggle | Mode::Momentary | Mode::Gate, Payload::Value(v)) => {
                ControlValue::Pressed(v >= 0x40)
            }
            (Mode::Relative { .. }, Payload::Press(_)) => return None,
            _ => return None,
        };

        Some(ControlEvent { action, value })
    }

    /// Binding matchant `key` : priorité à la couche courante (shift ou
    /// non), repli sur la couche de base si aucun binding shifté n'existe.
    fn find_binding(&self, key: InputSpec) -> Option<usize> {
        let mut fallback = None;
        for (i, control) in self.controls.iter().enumerate() {
            if control.input != key {
                continue;
            }
            if control.shift == self.shift {
                return Some(i);
            }
            if !control.shift {
                fallback = Some(i);
            }
        }
        fallback
    }
}

enum Payload {
    Press(bool),
    Value(u8),
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapping::{Curve, Deck, RelativeEncoding};

    fn hercules() -> MappingEngine {
        let mapping: Mapping = include_str!("../../../mappings/hercules_inpulse_200_mk2.ron")
            .parse()
            .unwrap();
        MappingEngine::new(&mapping)
    }

    fn jog_defaults() -> mapping::JogConfig {
        mapping::JogConfig {
            ticks_per_rev: 720,
            touch_scratch: true,
            bend_sensitivity: 0.3,
            release_ramp_ms: 100,
            platter_rpm: 100.0 / 3.0,
            velocity_window_ms: 15.0,
            scratch_smoothing_ms: 5.0,
            bend_return_ms: 150.0,
        }
    }

    #[test]
    fn table_hercules_transport_et_mixage() {
        let mut engine = hercules();

        // Play A : toggle.
        assert_eq!(
            engine.translate(&[0x91, 0x07, 0x7F]),
            Some(ControlEvent {
                action: Action::Play { deck: Deck::A },
                value: ControlValue::Toggled(true)
            })
        );
        // Relâchement d'un toggle : silencieux.
        assert_eq!(engine.translate(&[0x91, 0x07, 0x00]), None);
        // Deuxième pression : retour à false.
        assert_eq!(
            engine.translate(&[0x91, 0x07, 0x7F]).unwrap().value,
            ControlValue::Toggled(false)
        );

        // Cue B : momentané, press et release.
        assert_eq!(
            engine.translate(&[0x92, 0x06, 0x7F]),
            Some(ControlEvent {
                action: Action::Cue { deck: Deck::B },
                value: ControlValue::Pressed(true)
            })
        );
        assert_eq!(
            engine.translate(&[0x92, 0x06, 0x00]).unwrap().value,
            ControlValue::Pressed(false)
        );
        // NoteOff explicite (0x82) équivaut au release.
        engine.translate(&[0x92, 0x06, 0x7F]).unwrap();
        assert_eq!(
            engine.translate(&[0x82, 0x06, 0x40]).unwrap().value,
            ControlValue::Pressed(false)
        );

        // Crossfader : absolu linéaire.
        let event = engine.translate(&[0xB0, 0x00, 127]).unwrap();
        assert_eq!(event.action, Action::CrossFader);
        assert_eq!(event.value, ControlValue::Absolute(1.0));

        // Volume A à mi-course.
        let event = engine.translate(&[0xB1, 0x00, 64]).unwrap();
        assert_eq!(event.action, Action::Volume { deck: Deck::A });
        let ControlValue::Absolute(v) = event.value else {
            panic!()
        };
        assert!((v - 64.0 / 127.0).abs() < 1e-6);

        // EQ basses B : courbe dB — butées à −26 et +6.
        let event = engine.translate(&[0xB2, 0x02, 0]).unwrap();
        assert_eq!(event.action, Action::EqLow { deck: Deck::B });
        assert_eq!(event.value, ControlValue::Absolute(-26.0));
        let event = engine.translate(&[0xB2, 0x02, 127]).unwrap();
        assert_eq!(event.value, ControlValue::Absolute(6.0));

        // Jog A avec touch : ticks relatifs en complément à deux.
        let event = engine.translate(&[0xB1, 0x0A, 0x01]).unwrap();
        assert_eq!(event.action, Action::JogTick { deck: Deck::A });
        assert_eq!(event.value, ControlValue::Relative(1));
        let event = engine.translate(&[0xB1, 0x0A, 0x7F]).unwrap();
        assert_eq!(event.value, ControlValue::Relative(-1));

        // Touch du jog B : gate.
        let event = engine.translate(&[0x92, 0x08, 0x7F]).unwrap();
        assert_eq!(event.action, Action::JogTouch { deck: Deck::B });
        assert_eq!(event.value, ControlValue::Pressed(true));

        // Message non mappé : ignoré sans erreur.
        assert_eq!(engine.translate(&[0x9A, 0x55, 0x7F]), None);
        // Message tronqué : ignoré sans panique.
        assert_eq!(engine.translate(&[0xB1]), None);
        assert_eq!(engine.translate(&[]), None);
    }

    #[test]
    fn couche_shift_generique() {
        // Mapping synthétique : le Hercules gère le shift en matériel, mais
        // le moteur doit supporter la couche déclarative (specs §5.2).
        let mapping = Mapping {
            name: "test".into(),
            device_match: vec!["test".into()],
            init: vec![],
            controls: vec![
                ControlBinding {
                    input: InputSpec::NoteOn { ch: 0, note: 1 },
                    shift: false,
                    action: Action::Shift,
                    mode: Mode::Gate,
                },
                ControlBinding {
                    input: InputSpec::NoteOn { ch: 0, note: 2 },
                    shift: false,
                    action: Action::Play { deck: Deck::A },
                    mode: Mode::Toggle,
                },
                ControlBinding {
                    input: InputSpec::NoteOn { ch: 0, note: 2 },
                    shift: true,
                    action: Action::Cue { deck: Deck::A },
                    mode: Mode::Momentary,
                },
                ControlBinding {
                    input: InputSpec::CC { ch: 0, cc: 3 },
                    shift: false,
                    action: Action::Volume { deck: Deck::A },
                    mode: Mode::Absolute {
                        curve: Curve::Linear,
                    },
                },
            ],
            jog: jog_defaults(),
        };
        let mut engine = MappingEngine::new(&mapping);

        // Sans shift : Play.
        assert_eq!(
            engine.translate(&[0x90, 2, 0x7F]).unwrap().action,
            Action::Play { deck: Deck::A }
        );

        // Shift tenu : le même bouton devient Cue.
        engine.translate(&[0x90, 1, 0x7F]).unwrap();
        assert!(engine.shift_active());
        assert_eq!(
            engine.translate(&[0x90, 2, 0x7F]).unwrap().action,
            Action::Cue { deck: Deck::A }
        );
        // Un contrôle sans variante shiftée retombe sur la couche de base.
        assert_eq!(
            engine.translate(&[0xB0, 3, 64]).unwrap().action,
            Action::Volume { deck: Deck::A }
        );

        // Shift relâché : retour à Play.
        engine.translate(&[0x90, 1, 0x00]).unwrap();
        assert!(!engine.shift_active());
        assert_eq!(
            engine.translate(&[0x90, 2, 0x7F]).unwrap().action,
            Action::Play { deck: Deck::A }
        );
    }

    #[test]
    fn encodage_signed_bit() {
        let mapping = Mapping {
            name: "t".into(),
            device_match: vec!["t".into()],
            init: vec![],
            controls: vec![ControlBinding {
                input: InputSpec::CC { ch: 0, cc: 1 },
                shift: false,
                action: Action::JogTick { deck: Deck::A },
                mode: Mode::Relative {
                    encoding: RelativeEncoding::SignedBit,
                },
            }],
            jog: jog_defaults(),
        };
        let mut engine = MappingEngine::new(&mapping);
        assert_eq!(
            engine.translate(&[0xB0, 1, 0x03]).unwrap().value,
            ControlValue::Relative(3)
        );
        assert_eq!(
            engine.translate(&[0xB0, 1, 0x43]).unwrap().value,
            ControlValue::Relative(-3)
        );
    }
}
