//! Textes de l'écran unique (specs §6.3) : par deck (titre, BPM, temps
//! restant, pitch, cue) et barre d'état (périphérique audio, contrôleur,
//! underruns, charge CPU audio, fps). `Text2d` en Inter + tokens du thème.

use bevy::prelude::*;
use bevy::sprite::Anchor;

use engine::SAMPLE_RATE;

use crate::fonts::UiFonts;
use crate::theme::{color, font, layout};
use crate::{Analyzers, AudioEngine, Decks, LastSnapshot, MidiRes, MixState};

/// Cadence de rafraîchissement des textes (les uniforms, eux, bougent à
/// chaque frame — le texte n'a pas besoin de plus).
const REFRESH_SECONDS: f32 = 0.25;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(Update, (update_texts, layout_texts));
    }
}

/// Bloc de texte d'un deck (0 = A, 1 = B).
#[derive(Component)]
struct DeckText(usize);

/// Barre d'état, en bas de fenêtre.
#[derive(Component)]
struct StatusText;

fn spawn_hud(mut commands: Commands, fonts: Res<UiFonts>) {
    for deck in 0..2 {
        let accent = if deck == 0 {
            color::DECK_A
        } else {
            color::DECK_B
        };
        commands.spawn((
            Text2d::new(if deck == 0 { "Deck A" } else { "Deck B" }),
            TextFont {
                font: fonts.text.clone().into(),
                font_size: FontSize::Px(font::TITLE),
                ..Default::default()
            },
            TextColor(accent),
            // Deck A en haut à gauche, deck B en haut à droite (bandeau).
            if deck == 0 {
                Anchor::TOP_LEFT
            } else {
                Anchor::TOP_RIGHT
            },
            Transform::default(),
            DeckText(deck),
        ));
    }
    commands.spawn((
        Text2d::new("—"),
        TextFont {
            font: fonts.text.clone().into(),
            font_size: FontSize::Px(font::CAPTION),
            ..Default::default()
        },
        TextColor(color::TEXT_MUTED),
        Anchor::BOTTOM_LEFT,
        Transform::default(),
        StatusText,
    ));
}

fn format_time(samples: u64) -> String {
    let seconds = samples / u64::from(SAMPLE_RATE);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[allow(clippy::too_many_arguments)] // système Bevy : un paramètre par ressource
fn update_texts(
    time: Res<Time>,
    snapshot: Res<LastSnapshot>,
    decks: Res<Decks>,
    mix: Res<MixState>,
    midi: Res<MidiRes>,
    analyzers: Res<Analyzers>,
    engine: Res<AudioEngine>,
    mut deck_texts: Query<(&mut Text2d, &DeckText), Without<StatusText>>,
    mut status_texts: Query<&mut Text2d, With<StatusText>>,
    mut accumulator: Local<f32>,
    mut smoothed_fps: Local<f32>,
) {
    let dt = time.delta_secs().max(1e-6);
    // FPS lissé, basé sur le temps réel (jamais sur un compteur de frames).
    *smoothed_fps = if *smoothed_fps <= 0.0 {
        1.0 / dt
    } else {
        *smoothed_fps * 0.95 + (1.0 / dt) * 0.05
    };

    *accumulator += dt;
    if *accumulator < REFRESH_SECONDS {
        return;
    }
    *accumulator = 0.0;

    for (mut text, deck) in &mut deck_texts {
        let i = deck.0;
        let snap = &snapshot.0.decks[i];
        let label = if i == 0 { "A" } else { "B" };
        let (name, bpm) = decks.tracks[i].as_ref().map_or(("—", String::new()), |t| {
            (
                t.name.as_str(),
                t.analysis
                    .as_ref()
                    .map_or_else(String::new, |a| format!("  {:.2} BPM", a.bpm)),
            )
        });
        let state = if snap.playing { "▶" } else { "⏸" };
        let cue = if snap.cue { "  CUE" } else { "" };
        let remaining = format_time(snap.track_frames.saturating_sub(snap.position_samples));
        text.0 = format!(
            "{label} {state}  {name}{bpm}\n{} / −{remaining}   pitch {:+.1} %   vol {:.0} %{cue}",
            format_time(snap.position_samples),
            mix.pitch[i] * 100.0,
            mix.volumes[i] * 100.0,
        );
    }

    let (device, channels, buffer) = {
        let eng = engine.0.lock().unwrap();
        (
            eng.info.device_name.clone(),
            eng.info.channels,
            eng.info.buffer_frames,
        )
    };
    let vu = analyzers.levels.map_or_else(String::new, |(_, peak)| {
        format!("   vu {:.2}", peak[0].max(peak[1]))
    });
    if let Ok(mut text) = status_texts.single_mut() {
        text.0 = format!(
            "{device} ({channels} canaux, buffer {})   MIDI {}   xf {:+.2}   master {:.2}   casque {:.2}/mix {:.2}{vu}   underruns {}   audio {:.0} %   {:.0} fps",
            buffer.map_or("?".into(), |b| b.to_string()),
            midi.controller.as_deref().unwrap_or("—"),
            mix.crossfader,
            mix.master,
            mix.headphone,
            mix.cue_mix,
            snapshot.0.underruns,
            snapshot.0.callback_load * 100.0,
            *smoothed_fps,
        );
    }
}

fn layout_texts(
    windows: Query<&Window>,
    mut deck_texts: Query<(&mut Transform, &DeckText), Without<StatusText>>,
    mut status_texts: Query<&mut Transform, With<StatusText>>,
) {
    let Ok(window) = windows.single() else { return };
    let (half_w, half_h) = (window.width() * 0.5, window.height() * 0.5);

    // Bandeau supérieur : deck A à gauche, deck B à droite.
    for (mut transform, deck) in &mut deck_texts {
        let x = if deck.0 == 0 {
            -half_w + layout::MARGIN
        } else {
            half_w - layout::MARGIN
        };
        transform.translation = Vec3::new(x, half_h - layout::MARGIN * 0.6, 2.0);
    }
    if let Ok(mut transform) = status_texts.single_mut() {
        transform.translation = Vec3::new(
            -half_w + layout::MARGIN,
            -half_h + layout::MARGIN * 0.6,
            2.0,
        );
    }
}
