//! Routage chemin court (specs §5.1) : événement de contrôle → commande
//! moteur, sans passer par le scheduler Bevy. Utilisé par le thread MIDI ;
//! l'UI recevra une copie des événements pour l'affichage.

use engine::dsp::{self, EqBand};
use engine::{Deck as EngineDeck, EngineCommand, SAMPLE_RATE};
use mapping::{Action, Deck};

use crate::translate::{ControlEvent, ControlValue};

/// Excursion pitch du fader matériel (±8 % — le sélecteur 8/16 % viendra
/// avec le feedback M5 ; le moteur clampe de toute façon à ±16 %).
const PITCH_RANGE: f64 = 0.08;

fn engine_deck(deck: Deck) -> EngineDeck {
    match deck {
        Deck::A => EngineDeck::A,
        Deck::B => EngineDeck::B,
    }
}

/// Traduit un événement en commande moteur. `None` pour les actions sans
/// effet audio direct : `Load` (traitée par l'UI), `Shift` (état interne du
/// moteur de mapping), jogs (modèle scratch/bend au M4).
pub fn to_engine_command(event: &ControlEvent) -> Option<EngineCommand> {
    use ControlValue as V;

    let command = match (event.action, event.value) {
        // Transport. `Toggled` vient d'un bouton en mode Toggle ; un bouton
        // momentané produit Play sur chaque pression.
        (Action::Play { deck }, V::Toggled(true) | V::Pressed(true)) => {
            EngineCommand::Play(engine_deck(deck))
        }
        (Action::Play { deck }, V::Toggled(false)) => EngineCommand::Pause(engine_deck(deck)),
        (Action::Play { .. }, V::Pressed(false)) => return None,

        (Action::Cue { deck }, V::Pressed(true)) => EngineCommand::CuePress(engine_deck(deck)),
        (Action::Cue { deck }, V::Pressed(false)) => EngineCommand::CueRelease(engine_deck(deck)),

        // Mixage.
        (Action::Volume { deck }, V::Absolute(v)) => {
            EngineCommand::SetDeckVolume(engine_deck(deck), v)
        }
        (Action::CrossFader, V::Absolute(v)) => EngineCommand::SetCrossfader(v * 2.0 - 1.0),
        (Action::MasterGain, V::Absolute(v)) => EngineCommand::SetMasterGain(v),

        // EQ : la courbe du mapping produit des dB, les coefficients sont
        // calculés ici, hors callback (specs §3.3).
        (Action::EqLow { deck }, V::Absolute(db)) => eq(deck, EqBand::Low, db),
        (Action::EqMid { deck }, V::Absolute(db)) => eq(deck, EqBand::Mid, db),
        (Action::EqHigh { deck }, V::Absolute(db)) => eq(deck, EqBand::High, db),

        // Pitch : fader 0–1 → vitesse autour de 1.0.
        (Action::Pitch { deck }, V::Absolute(v)) => EngineCommand::SetPitch(
            engine_deck(deck),
            1.0 + (f64::from(v) * 2.0 - 1.0) * PITCH_RANGE,
        ),

        // Casque.
        (Action::HeadphoneCue { deck }, V::Toggled(on) | V::Pressed(on)) => {
            EngineCommand::SetCueEnabled(engine_deck(deck), on)
        }
        (Action::CueMix, V::Absolute(v)) => EngineCommand::SetCueMix(v),
        (Action::HeadphoneGain, V::Absolute(v)) => EngineCommand::SetHeadphoneGain(v),

        // UI (Load), état interne (Shift), jogs (M4).
        _ => return None,
    };
    Some(command)
}

fn eq(deck: Deck, band: EqBand, gain_db: f32) -> EngineCommand {
    EngineCommand::SetEq(
        engine_deck(deck),
        band,
        dsp::eq_coeffs(band, f64::from(gain_db), f64::from(SAMPLE_RATE)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_chemin_court_couvre_les_controles_critiques() {
        let xf = to_engine_command(&ControlEvent {
            action: Action::CrossFader,
            value: ControlValue::Absolute(1.0),
        });
        assert!(matches!(xf, Some(EngineCommand::SetCrossfader(v)) if (v - 1.0).abs() < 1e-6));

        let pitch = to_engine_command(&ControlEvent {
            action: Action::Pitch { deck: Deck::A },
            value: ControlValue::Absolute(0.0),
        });
        assert!(
            matches!(pitch, Some(EngineCommand::SetPitch(EngineDeck::A, s)) if (s - 0.92).abs() < 1e-9)
        );

        let eq_kill = to_engine_command(&ControlEvent {
            action: Action::EqLow { deck: Deck::B },
            value: ControlValue::Absolute(-26.0),
        });
        assert!(matches!(
            eq_kill,
            Some(EngineCommand::SetEq(EngineDeck::B, EqBand::Low, _))
        ));

        let cue = to_engine_command(&ControlEvent {
            action: Action::Cue { deck: Deck::A },
            value: ControlValue::Pressed(true),
        });
        assert!(matches!(cue, Some(EngineCommand::CuePress(EngineDeck::A))));

        // Sans effet moteur : Load, Shift, jogs (M4).
        for action in [
            Action::Load { deck: Deck::A },
            Action::Shift,
            Action::JogTick { deck: Deck::A },
            Action::JogBend { deck: Deck::A },
            Action::JogTouch { deck: Deck::A },
        ] {
            let event = ControlEvent {
                action,
                value: ControlValue::Relative(1),
            };
            assert!(to_engine_command(&event).is_none(), "{action:?}");
        }
    }
}
