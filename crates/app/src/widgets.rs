//! Widgets souris de l'écran unique (specs §6.3) : par deck, boutons
//! play/cue/PFL/load et sliders volume/pitch/EQ ; au centre, crossfader,
//! mix cue↔master, gain casque et master autour des VU.
//!
//! Rendu : quads `ColorMaterial` + étiquettes `Text2d` (Inter), couleurs et
//! espacements du thème. Interaction : hit-testing manuel curseur → monde
//! (aucune dépendance à un backend de picking). Chaque interaction émet les
//! mêmes `mapping::Action` que le MIDI (specs §6.4) via
//! [`crate::emit_control`].

use bevy::prelude::*;
use bevy::sprite::Anchor;

use engine::dsp::eq::{EQ_MAX_DB, EQ_MIN_DB};

use crate::fonts::UiFonts;
use crate::theme::{color, font, layout};
use crate::{AudioEngine, LastSnapshot, LoadSender, MixState, PITCH_RANGE, emit_control, picker};

pub struct WidgetsPlugin;

impl Plugin for WidgetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PointerGrab>()
            .add_systems(Startup, spawn_widgets)
            .add_systems(Update, (interact, place_widgets, update_visuals).chain());
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ButtonKind {
    Play(usize),
    Cue(usize),
    Pfl(usize),
    Load(usize),
}

#[derive(Clone, Copy, PartialEq)]
enum SliderKind {
    Volume(usize),
    Pitch(usize),
    /// (deck, bande 0..3)
    Eq(usize, usize),
    CrossFader,
    CueMix,
    Headphone,
    Master,
}

impl SliderKind {
    fn horizontal(self) -> bool {
        matches!(self, SliderKind::CrossFader)
    }
}

#[derive(Component, Clone, Copy)]
enum Widget {
    Button(ButtonKind),
    Slider(SliderKind),
}

/// Curseur d'un slider (positionné selon la valeur courante).
#[derive(Component)]
struct Thumb(SliderKind);

/// Étiquette d'un widget (positionnée par rapport à son slot).
#[derive(Component)]
struct Label(Widget);

#[derive(Resource, Default)]
struct PointerGrab {
    slider: Option<SliderKind>,
    button: Option<ButtonKind>,
}

/// Valeur normalisée 0..1 d'un slider depuis l'état d'affichage.
fn slider_value(kind: SliderKind, mix: &MixState) -> f32 {
    match kind {
        SliderKind::Volume(i) => mix.volumes[i],
        SliderKind::Pitch(i) => mix.pitch[i] / PITCH_RANGE * 0.5 + 0.5,
        SliderKind::Eq(i, band) => {
            (mix.eq_db[i][band] - EQ_MIN_DB as f32) / (EQ_MAX_DB - EQ_MIN_DB) as f32
        }
        SliderKind::CrossFader => mix.crossfader * 0.5 + 0.5,
        SliderKind::CueMix => mix.cue_mix,
        SliderKind::Headphone => mix.headphone * 0.5,
        SliderKind::Master => mix.master * 0.5,
    }
}

/// Événement de contrôle correspondant à une valeur normalisée.
fn slider_event(kind: SliderKind, t: f32) -> (mapping::Action, midi::ControlValue) {
    use mapping::Action as A;
    use midi::ControlValue as V;
    let deck = |i: usize| {
        if i == 0 {
            mapping::Deck::A
        } else {
            mapping::Deck::B
        }
    };
    match kind {
        SliderKind::Volume(i) => (A::Volume { deck: deck(i) }, V::Absolute(t)),
        SliderKind::Pitch(i) => (A::Pitch { deck: deck(i) }, V::Absolute(t)),
        SliderKind::Eq(i, band) => {
            let db = EQ_MIN_DB as f32 + t * (EQ_MAX_DB - EQ_MIN_DB) as f32;
            let action = match band {
                0 => A::EqLow { deck: deck(i) },
                1 => A::EqMid { deck: deck(i) },
                _ => A::EqHigh { deck: deck(i) },
            };
            (action, V::Absolute(db))
        }
        SliderKind::CrossFader => (A::CrossFader, V::Absolute(t)),
        SliderKind::CueMix => (A::CueMix, V::Absolute(t)),
        SliderKind::Headphone => (A::HeadphoneGain, V::Absolute(t * 2.0)),
        SliderKind::Master => (A::MasterGain, V::Absolute(t * 2.0)),
    }
}

/// Slot (centre, taille) d'un widget pour une fenêtre donnée — source
/// unique pour le layout, le hit-testing et les étiquettes.
fn slot(widget: Widget, window: &Window) -> (Vec2, Vec2) {
    let (hw, hh) = (window.width() * 0.5, window.height() * 0.5);
    let column_x = |deck: usize| {
        let x = hw - layout::MARGIN - layout::SIDE_COLUMN_PX * 0.5;
        if deck == 0 { -x } else { x }
    };
    match widget {
        Widget::Button(kind) => {
            let (deck, row) = match kind {
                ButtonKind::Play(d) => (d, 0.0),
                ButtonKind::Cue(d) => (d, 1.0),
                ButtonKind::Pfl(d) => (d, 2.0),
                ButtonKind::Load(d) => (d, 3.0),
            };
            (
                Vec2::new(column_x(deck), hh - 56.0 - row * 46.0),
                Vec2::new(96.0, 34.0),
            )
        }
        Widget::Slider(kind) => match kind {
            SliderKind::Volume(d) => (
                Vec2::new(column_x(d) - 26.0, hh - 320.0),
                Vec2::new(14.0, 140.0),
            ),
            SliderKind::Pitch(d) => (
                Vec2::new(column_x(d) + 26.0, hh - 320.0),
                Vec2::new(14.0, 140.0),
            ),
            SliderKind::Eq(d, band) => (
                Vec2::new(column_x(d) + (band as f32 - 1.0) * 26.0, hh - 490.0),
                Vec2::new(12.0, 100.0),
            ),
            SliderKind::CrossFader => (Vec2::new(-170.0, 0.0), Vec2::new(200.0, 12.0)),
            SliderKind::CueMix => (Vec2::new(60.0, 0.0), Vec2::new(10.0, 90.0)),
            SliderKind::Headphone => (Vec2::new(90.0, 0.0), Vec2::new(10.0, 90.0)),
            SliderKind::Master => (Vec2::new(120.0, 0.0), Vec2::new(10.0, 90.0)),
        },
    }
}

fn label_text(widget: Widget) -> &'static str {
    match widget {
        Widget::Button(ButtonKind::Play(_)) => "PLAY",
        Widget::Button(ButtonKind::Cue(_)) => "CUE",
        Widget::Button(ButtonKind::Pfl(_)) => "PFL",
        Widget::Button(ButtonKind::Load(_)) => "LOAD",
        Widget::Slider(SliderKind::Volume(_)) => "vol",
        Widget::Slider(SliderKind::Pitch(_)) => "pitch",
        Widget::Slider(SliderKind::Eq(_, 0)) => "low",
        Widget::Slider(SliderKind::Eq(_, 1)) => "mid",
        Widget::Slider(SliderKind::Eq(_, _)) => "high",
        Widget::Slider(SliderKind::CrossFader) => "crossfader",
        Widget::Slider(SliderKind::CueMix) => "cue",
        Widget::Slider(SliderKind::Headphone) => "phones",
        Widget::Slider(SliderKind::Master) => "master",
    }
}

fn all_widgets() -> Vec<Widget> {
    let mut widgets = Vec::new();
    for deck in 0..2 {
        widgets.push(Widget::Button(ButtonKind::Play(deck)));
        widgets.push(Widget::Button(ButtonKind::Cue(deck)));
        widgets.push(Widget::Button(ButtonKind::Pfl(deck)));
        widgets.push(Widget::Button(ButtonKind::Load(deck)));
        widgets.push(Widget::Slider(SliderKind::Volume(deck)));
        widgets.push(Widget::Slider(SliderKind::Pitch(deck)));
        for band in 0..3 {
            widgets.push(Widget::Slider(SliderKind::Eq(deck, band)));
        }
    }
    widgets.extend([
        Widget::Slider(SliderKind::CrossFader),
        Widget::Slider(SliderKind::CueMix),
        Widget::Slider(SliderKind::Headphone),
        Widget::Slider(SliderKind::Master),
    ]);
    widgets
}

fn spawn_widgets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fonts: Res<UiFonts>,
) {
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    for widget in all_widgets() {
        commands.spawn((
            Mesh2d(quad.clone()),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(color::WIDGET_BG))),
            Transform::default(),
            widget,
        ));
        if let Widget::Slider(kind) = widget {
            commands.spawn((
                Mesh2d(quad.clone()),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(color::THUMB))),
                Transform::default(),
                Thumb(kind),
            ));
        }
        commands.spawn((
            Text2d::new(label_text(widget)),
            TextFont {
                font: fonts.text.clone().into(),
                font_size: FontSize::Px(font::CAPTION),
                ..Default::default()
            },
            TextColor(color::TEXT_MUTED),
            Anchor::CENTER,
            Transform::default(),
            Label(widget),
        ));
    }
}

/// Positionne quads, curseurs et étiquettes depuis `slot` (recalcul par
/// frame : quelques multiplications, aucune géométrie touchée).
#[allow(clippy::type_complexity)] // filtres de requêtes Bevy disjointes
fn place_widgets(
    windows: Query<&Window>,
    mix: Res<MixState>,
    mut quads: Query<(&mut Transform, &Widget), (Without<Thumb>, Without<Label>)>,
    mut thumbs: Query<(&mut Transform, &Thumb), (Without<Widget>, Without<Label>)>,
    mut labels: Query<(&mut Transform, &Label), (Without<Widget>, Without<Thumb>)>,
) {
    let Ok(window) = windows.single() else { return };

    for (mut transform, widget) in &mut quads {
        let (center, size) = slot(*widget, window);
        transform.translation = center.extend(3.0);
        transform.scale = Vec3::new(size.x, size.y, 1.0);
    }
    for (mut transform, thumb) in &mut thumbs {
        let (center, size) = slot(Widget::Slider(thumb.0), window);
        let t = slider_value(thumb.0, &mix) - 0.5;
        let (offset, thumb_size) = if thumb.0.horizontal() {
            (Vec2::new(t * size.x, 0.0), Vec2::new(8.0, size.y + 8.0))
        } else {
            (Vec2::new(0.0, t * size.y), Vec2::new(size.x + 8.0, 8.0))
        };
        transform.translation = (center + offset).extend(4.0);
        transform.scale = Vec3::new(thumb_size.x, thumb_size.y, 1.0);
    }
    for (mut transform, label) in &mut labels {
        let (center, size) = slot(label.0, window);
        let position = match label.0 {
            Widget::Button(_) => center,
            Widget::Slider(_) => center - Vec2::new(0.0, size.y * 0.5 + 12.0),
        };
        transform.translation = position.extend(5.0);
    }
}

/// Hit-testing manuel + émission des `Action` (specs §6.4).
#[allow(clippy::too_many_arguments)] // système Bevy : un paramètre par ressource
fn interact(
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    widgets: Query<(&Widget, &Transform)>,
    engine: Res<AudioEngine>,
    mut mix: ResMut<MixState>,
    snapshot: Res<LastSnapshot>,
    mut grab: ResMut<PointerGrab>,
    load_tx: Res<LoadSender>,
) {
    use mapping::Action as A;
    use midi::ControlValue as V;

    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let point = Vec2::new(
        cursor.x - window.width() * 0.5,
        window.height() * 0.5 - cursor.y,
    );
    let deck = |i: usize| {
        if i == 0 {
            mapping::Deck::A
        } else {
            mapping::Deck::B
        }
    };

    let mut eng = engine.0.lock().unwrap();
    let mix = &mut *mix;

    if mouse.just_pressed(MouseButton::Left) {
        let hit = widgets.iter().find(|(_, transform)| {
            let half = transform.scale.truncate() * 0.5;
            (point - transform.translation.truncate())
                .abs()
                .cmple(half)
                .all()
        });
        match hit.map(|(w, _)| *w) {
            Some(Widget::Button(kind)) => {
                grab.button = Some(kind);
                match kind {
                    ButtonKind::Play(i) => {
                        let playing = snapshot.0.decks[i].playing;
                        emit_control(
                            &mut eng,
                            mix,
                            A::Play { deck: deck(i) },
                            V::Toggled(!playing),
                        );
                    }
                    ButtonKind::Cue(i) => {
                        emit_control(&mut eng, mix, A::Cue { deck: deck(i) }, V::Pressed(true));
                    }
                    ButtonKind::Pfl(i) => {
                        let cue = mix.cue[i];
                        emit_control(
                            &mut eng,
                            mix,
                            A::HeadphoneCue { deck: deck(i) },
                            V::Toggled(!cue),
                        );
                    }
                    ButtonKind::Load(i) => {
                        picker::open(
                            if i == 0 {
                                engine::Deck::A
                            } else {
                                engine::Deck::B
                            },
                            load_tx.0.clone(),
                        );
                    }
                }
            }
            Some(Widget::Slider(kind)) => grab.slider = Some(kind),
            None => {}
        }
    }

    // Glissement d'un slider : valeur depuis la position du curseur.
    if mouse.pressed(MouseButton::Left)
        && let Some(kind) = grab.slider
        && let Some((_, transform)) = widgets
            .iter()
            .find(|(w, _)| matches!(w, Widget::Slider(k) if *k == kind))
    {
        let center = transform.translation.truncate();
        let size = transform.scale.truncate();
        let t = if kind.horizontal() {
            (point.x - center.x) / size.x + 0.5
        } else {
            (point.y - center.y) / size.y + 0.5
        }
        .clamp(0.0, 1.0);
        let (action, value) = slider_event(kind, t);
        emit_control(&mut eng, mix, action, value);
    }

    if mouse.just_released(MouseButton::Left) {
        if let Some(ButtonKind::Cue(i)) = grab.button {
            emit_control(&mut eng, mix, A::Cue { deck: deck(i) }, V::Pressed(false));
        }
        grab.button = None;
        grab.slider = None;
    }
}

/// Couleurs d'état des boutons (accent du deck quand actif).
fn update_visuals(
    snapshot: Res<LastSnapshot>,
    mix: Res<MixState>,
    grab: Res<PointerGrab>,
    widgets: Query<(&Widget, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (widget, material) in &widgets {
        let Widget::Button(kind) = widget else {
            continue;
        };
        let (deck, active) = match *kind {
            ButtonKind::Play(i) => (i, snapshot.0.decks[i].playing),
            ButtonKind::Cue(i) => (i, grab.button == Some(ButtonKind::Cue(i))),
            ButtonKind::Pfl(i) => (i, mix.cue[i]),
            ButtonKind::Load(i) => (i, false),
        };
        let accent = if deck == 0 {
            color::DECK_A
        } else {
            color::DECK_B
        };
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.color = if active { accent } else { color::WIDGET_BG };
        }
    }
}
