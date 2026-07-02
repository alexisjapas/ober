//! Panneau utilitaire `bevy_egui` (specs §6.1) : préférences et
//! diagnostics, **jamais visible pendant une session de mix normale** —
//! bascule avec `F12`. Stylé depuis les tokens du thème (§6.2).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

use crate::theme;
use crate::waveform::WaveZoom;
use crate::{AudioEngine, LastSnapshot, LoadSender, MidiRes, picker};

pub struct PanelPlugin;

#[derive(Resource, Default)]
struct PanelVisible(bool);

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<PanelVisible>()
            .add_systems(Update, toggle_panel)
            .add_systems(EguiPrimaryContextPass, draw_panel);
    }
}

fn toggle_panel(keys: Res<ButtonInput<KeyCode>>, mut visible: ResMut<PanelVisible>) {
    if keys.just_pressed(KeyCode::F12) {
        visible.0 = !visible.0;
    }
}

fn egui_color(color: Color) -> egui::Color32 {
    let srgba = color.to_srgba();
    egui::Color32::from_rgb(
        (srgba.red * 255.0) as u8,
        (srgba.green * 255.0) as u8,
        (srgba.blue * 255.0) as u8,
    )
}

fn draw_panel(
    mut contexts: EguiContexts,
    visible: Res<PanelVisible>,
    engine: Res<AudioEngine>,
    snapshot: Res<LastSnapshot>,
    midi: Res<MidiRes>,
    mut zoom: ResMut<WaveZoom>,
    load_tx: Res<LoadSender>,
) -> Result {
    if !visible.0 {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;

    // Cohérence visuelle avec le design system (§6.2).
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = egui_color(theme::color::SURFACE);
    visuals.panel_fill = egui_color(theme::color::SURFACE);
    visuals.override_text_color = Some(egui_color(theme::color::TEXT_PRIMARY));
    visuals.selection.bg_fill = egui_color(theme::color::DECK_A);
    ctx.set_visuals(visuals);

    egui::Window::new("ober — préférences & diagnostics (F12)").show(ctx, |ui| {
        ui.heading("Préférences");
        let mut seconds = zoom.seconds;
        ui.add(
            egui::Slider::new(&mut seconds, 2.0..=180.0)
                .logarithmic(true)
                .text("fenêtre waveform (s)"),
        );
        if seconds != zoom.seconds {
            zoom.seconds = seconds;
        }
        ui.horizontal(|ui| {
            ui.label("Dialogue système (rfd) :");
            if ui.button("→ deck A").clicked() {
                picker::open(engine::Deck::A, load_tx.0.clone());
            }
            if ui.button("→ deck B").clicked() {
                picker::open(engine::Deck::B, load_tx.0.clone());
            }
        });

        ui.separator();
        ui.heading("Diagnostics");
        {
            let eng = engine.0.lock().unwrap();
            ui.label(format!(
                "Sortie : {} — {} canaux, buffer {} ({} ms)",
                eng.info.device_name,
                eng.info.channels,
                eng.info.buffer_frames.map_or("?".into(), |b| b.to_string()),
                eng.info
                    .buffer_latency_ms()
                    .map_or("?".into(), |ms| format!("{ms:.1}")),
            ));
        }
        ui.label(format!(
            "MIDI : {}",
            midi.controller.as_deref().unwrap_or("aucun contrôleur")
        ));
        ui.label(format!(
            "Underruns : {} — charge callback : {:.1} %",
            snapshot.0.underruns,
            snapshot.0.callback_load * 100.0
        ));
        for (i, deck) in snapshot.0.decks.iter().enumerate() {
            ui.label(format!(
                "Deck {} : pos {} — vitesse {:.3} — cue {}",
                if i == 0 { "A" } else { "B" },
                deck.position_samples,
                deck.speed,
                deck.cue_point_samples,
            ));
        }
    });
    Ok(())
}
