//! Power management (specs §6.5): the continuous game loop is the intended
//! behavior during playback; at idle (no deck playing, no interaction for
//! more than 5 s) the target framerate drops to 10 fps via
//! `WinitSettings`. Immediate return to the native framerate on any
//! interaction or playback — the audio thread is never affected.
//!
//! An interaction is an *intent*, whatever its source (specs §6.4): winit
//! inputs (keyboard, mouse) are read here; MIDI controller events don't go
//! through winit, so the system draining them marks [`ControlActivity`].

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
        .init_resource::<ControlActivity>()
        .add_systems(Update, power_management);
    }
}

#[derive(Resource, Default)]
struct Activity {
    seconds_since: f32,
    idle: bool,
}

/// Interaction signal for control sources that bypass winit (the MIDI
/// controller, specs §5.1): faders, knobs, buttons and encoders must wake
/// the UI from idle exactly like the keyboard and mouse do. The flag
/// persists until `power_management` consumes it, so marker systems can run
/// in any order relative to it.
#[derive(Resource, Default)]
pub struct ControlActivity {
    marked: bool,
}

impl ControlActivity {
    pub fn mark(&mut self) {
        self.marked = true;
    }

    fn take(&mut self) -> bool {
        std::mem::take(&mut self.marked)
    }
}

#[allow(clippy::too_many_arguments)] // système Bevy : un paramètre par source d'activité
fn power_management(
    time: Res<Time>,
    snapshot: Res<LastSnapshot>,
    keys: Res<ButtonInput<KeyCode>>,
    mut motion: MessageReader<MouseMotion>,
    mut wheel: MessageReader<MouseWheel>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut controls: ResMut<ControlActivity>,
    mut activity: ResMut<Activity>,
    mut settings: ResMut<WinitSettings>,
) {
    // Consumed unconditionally: the flag must never survive a frame where
    // another source already reset the idle timer.
    let midi_activity = controls.take();
    let interacting = midi_activity
        || keys.get_pressed().next().is_some()
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
