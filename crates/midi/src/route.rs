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

        // Seek relatif en secondes signées (clavier/UI, encodeurs M3+).
        (Action::Seek { deck }, V::Relative(seconds)) => EngineCommand::SeekRelative(
            engine_deck(deck),
            i64::from(seconds) * i64::from(SAMPLE_RATE),
        ),

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

        // Jogs (specs §3.5) : le moteur choisit scratch ou bend selon le
        // touch — les deux CC (bord/surface) portent les mêmes ticks.
        (Action::JogTouch { deck }, V::Pressed(on)) => {
            EngineCommand::JogTouch(engine_deck(deck), on)
        }
        (Action::JogTick { deck } | Action::JogBend { deck }, V::Relative(n)) => {
            EngineCommand::JogTicks(engine_deck(deck), n)
        }

        // UI (Load), état interne (Shift).
        _ => return None,
    };
    Some(command)
}

/// Convertit les paramètres de jog du mapping RON (unités « humaines ») en
/// paramètres moteur (SI). Envoyé au moteur à l'initialisation du thread
/// MIDI — rien n'est codé en dur (specs §3.5).
pub fn jog_params(config: &mapping::JogConfig) -> engine::JogParams {
    engine::JogParams {
        ticks_per_rev: f64::from(config.ticks_per_rev),
        touch_scratch: config.touch_scratch,
        bend_sensitivity: f64::from(config.bend_sensitivity),
        release_ramp: f64::from(config.release_ramp_ms) / 1000.0,
        platter_rev_per_s: f64::from(config.platter_rpm) / 60.0,
        velocity_window: f64::from(config.velocity_window_ms) / 1000.0,
        scratch_smoothing: f64::from(config.scratch_smoothing_ms) / 1000.0,
        bend_return: f64::from(config.bend_return_ms) / 1000.0,
    }
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

        // Jogs : ticks unifiés, touch transmis.
        let tick = to_engine_command(&ControlEvent {
            action: Action::JogTick { deck: Deck::A },
            value: ControlValue::Relative(-1),
        });
        assert!(matches!(
            tick,
            Some(EngineCommand::JogTicks(EngineDeck::A, -1))
        ));
        let bend = to_engine_command(&ControlEvent {
            action: Action::JogBend { deck: Deck::B },
            value: ControlValue::Relative(2),
        });
        assert!(matches!(
            bend,
            Some(EngineCommand::JogTicks(EngineDeck::B, 2))
        ));
        let touch = to_engine_command(&ControlEvent {
            action: Action::JogTouch { deck: Deck::A },
            value: ControlValue::Pressed(true),
        });
        assert!(matches!(
            touch,
            Some(EngineCommand::JogTouch(EngineDeck::A, true))
        ));

        // Sans effet moteur : Load, Shift.
        for action in [Action::Load { deck: Deck::A }, Action::Shift] {
            let event = ControlEvent {
                action,
                value: ControlValue::Pressed(true),
            };
            assert!(to_engine_command(&event).is_none(), "{action:?}");
        }
    }

    #[test]
    fn les_parametres_de_jog_viennent_du_mapping() {
        let mapping: mapping::Mapping =
            include_str!("../../../mappings/hercules_inpulse_200_mk2.ron")
                .parse()
                .unwrap();
        let params = jog_params(&mapping.jog);
        assert_eq!(params.ticks_per_rev, 720.0);
        assert!(params.touch_scratch);
        assert!((params.release_ramp - 0.1).abs() < 1e-9);
        assert!((params.platter_rev_per_s - 100.0 / 180.0).abs() < 1e-6);
    }
}
