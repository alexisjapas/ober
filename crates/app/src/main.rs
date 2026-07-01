//! Binaire Bevy : UI, orchestration, plugins (specs §6). Seule crate du
//! workspace autorisée à dépendre de Bevy (frontière §1.4, vérifiée en CI).
//!
//! M0 : fenêtre vide. Les plugins arrivent avec les jalons : moteur audio et
//! contrôles clavier (M1), MIDI (M3), waveforms shader + design system +
//! mode idle basse consommation (M6).

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("dj-mix {}", env!("CARGO_PKG_VERSION")),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .run();
}
