//! Gestion de l'énergie (specs §6.5) : la game loop continue est le
//! comportement voulu en lecture ; à l'idle (aucun deck en lecture, aucune
//! interaction depuis > 5 s), le framerate cible tombe à 10 fps via
//! `WinitSettings`. Retour immédiat au framerate natif à la moindre
//! interaction ou lecture — le thread audio n'est jamais affecté.

use std::time::Duration;

use bevy::input::mouse::{MouseButtonInput, MouseMotion, MouseWheel};
use bevy::prelude::*;
use bevy::winit::{UpdateMode, WinitSettings};

use crate::LastSnapshot;

const IDLE_AFTER_SECONDS: f32 = 5.0;
/// 10 fps à l'idle (specs §6.5).
const IDLE_FRAME: Duration = Duration::from_millis(100);

pub struct PowerPlugin;

impl Plugin for PowerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WinitSettings {
            focused_mode: UpdateMode::Continuous,
            unfocused_mode: UpdateMode::Continuous,
        })
        .init_resource::<Activity>()
        .add_systems(Update, power_management);
    }
}

#[derive(Resource, Default)]
struct Activity {
    seconds_since: f32,
    idle: bool,
}

#[allow(clippy::too_many_arguments)] // système Bevy : un paramètre par source d'activité
fn power_management(
    time: Res<Time>,
    snapshot: Res<LastSnapshot>,
    keys: Res<ButtonInput<KeyCode>>,
    mut motion: MessageReader<MouseMotion>,
    mut wheel: MessageReader<MouseWheel>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut activity: ResMut<Activity>,
    mut settings: ResMut<WinitSettings>,
) {
    let interacting = keys.get_pressed().next().is_some()
        || motion.read().next().is_some()
        || wheel.read().next().is_some()
        || buttons.read().next().is_some();
    let playing = snapshot.0.decks.iter().any(|d| d.playing || d.speed != 0.0);

    if interacting || playing {
        activity.seconds_since = 0.0;
        if activity.idle {
            activity.idle = false;
            settings.focused_mode = UpdateMode::Continuous;
            settings.unfocused_mode = UpdateMode::Continuous;
        }
        return;
    }

    activity.seconds_since += time.delta_secs();
    if !activity.idle && activity.seconds_since > IDLE_AFTER_SECONDS {
        activity.idle = true;
        settings.focused_mode = UpdateMode::reactive_low_power(IDLE_FRAME);
        settings.unfocused_mode = UpdateMode::reactive_low_power(IDLE_FRAME);
    }
}
