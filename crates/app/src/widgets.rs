//! Widgets souris de l'écran unique (specs §6.3), disposés dans la bande de
//! contrôles entre les deux waveforms :
//!
//! ```text
//! [ deck A: PLAY CUE PFL LOAD ]  [ cue φ VU master ]  [ deck B: … ]
//! [        vol pitch l m h    ]  [   crossfader    ]  [    …      ]
//! ```
//!
//! TOUT le layout est exprimé en fractions de la fenêtre (`theme::layout`) :
//! il se réarrange au redimensionnement. Rendu : panneaux de fond, pistes de
//! sliders avec jauge de remplissage et curseur, étiquettes Inter — couleurs
//! du thème uniquement. Interaction : hit-testing manuel curseur → monde ;
//! chaque geste émet les mêmes `mapping::Action` que le MIDI (specs §6.4)
//! via [`crate::emit_control`].

use bevy::prelude::*;
use bevy::sprite::Anchor;

use engine::dsp::eq::{EQ_MAX_DB, EQ_MIN_DB};

use crate::browser::Browser;
use crate::fonts::UiFonts;
use crate::theme::{color, font, layout};
use crate::{AudioEngine, LastSnapshot, MixState, PITCH_RANGE, emit_control};

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

    /// Le crossfader est bipolaire : la jauge part du centre.
    fn bipolar(self) -> bool {
        matches!(self, SliderKind::CrossFader | SliderKind::Pitch(_))
    }

    fn accent(self) -> Color {
        match self {
            SliderKind::Volume(d) | SliderKind::Pitch(d) | SliderKind::Eq(d, _) => {
                if d == 0 {
                    color::DECK_A
                } else {
                    color::DECK_B
                }
            }
            _ => color::TEXT_MUTED,
        }
    }
}

#[derive(Component, Clone, Copy)]
enum Widget {
    Button(ButtonKind),
    Slider(SliderKind),
}

/// Jauge de remplissage d'un slider.
#[derive(Component)]
struct Fill(SliderKind);

/// Curseur d'un slider.
#[derive(Component)]
struct Thumb(SliderKind);

/// Étiquette d'un widget.
#[derive(Component)]
struct Label(Widget);

/// Panneaux de fond : 0 = deck A, 1 = deck B, 2 = mixer central.
#[derive(Component)]
struct Panel(usize);

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

/// Zones horizontales de la bande de contrôles : (centre x, largeur).
fn deck_zone(deck: usize, width: f32) -> (f32, f32) {
    let hw = width * 0.5;
    let inner = width * 0.20; // bord intérieur (début de la zone mixer)
    let zone_width = hw - inner - layout::MARGIN;
    let center = -(inner + zone_width * 0.5);
    if deck == 0 {
        (center, zone_width)
    } else {
        (-center, zone_width)
    }
}

/// Slot (centre, taille) d'un widget — source unique pour le layout, le
/// hit-testing, les jauges et les étiquettes. Tout en fractions de fenêtre.
fn slot(widget: Widget, window: &Window) -> (Vec2, Vec2) {
    let (w, h) = (window.width(), window.height());
    let bands = layout::bands(w, h);
    let (cy, ch) = (bands.controls_center, bands.controls_height);

    match widget {
        Widget::Button(kind) => {
            let (deck, index) = match kind {
                ButtonKind::Play(d) => (d, 0.0),
                ButtonKind::Cue(d) => (d, 1.0),
                ButtonKind::Pfl(d) => (d, 2.0),
                ButtonKind::Load(d) => (d, 3.0),
            };
            let (zx, zw) = deck_zone(deck, w);
            let button_w = (zw - 3.0 * layout::GAP) / 4.0;
            let x = zx - zw * 0.5 + button_w * 0.5 + index * (button_w + layout::GAP);
            let y = cy + ch * 0.30;
            (
                Vec2::new(x, y),
                Vec2::new(button_w, (ch * 0.20).clamp(24.0, 42.0)),
            )
        }
        Widget::Slider(kind) => match kind {
            SliderKind::Volume(_) | SliderKind::Pitch(_) | SliderKind::Eq(..) => {
                let (deck, index) = match kind {
                    SliderKind::Volume(d) => (d, 0.0),
                    SliderKind::Pitch(d) => (d, 1.0),
                    SliderKind::Eq(d, band) => (d, 2.0 + band as f32),
                    _ => unreachable!(),
                };
                let (zx, zw) = deck_zone(deck, w);
                let x = zx - zw * 0.5 + zw * (index + 0.5) / 5.0;
                let y = cy - ch * 0.17;
                (
                    Vec2::new(x, y),
                    Vec2::new(10.0, (ch * 0.45).clamp(56.0, 180.0)),
                )
            }
            SliderKind::CrossFader => (
                Vec2::new(0.0, cy - ch * 0.30),
                Vec2::new((w * 0.24).clamp(150.0, 380.0), 10.0),
            ),
            SliderKind::CueMix | SliderKind::Headphone | SliderKind::Master => {
                let x = match kind {
                    SliderKind::CueMix => -w * 0.075,
                    SliderKind::Headphone => -w * 0.045,
                    _ => w * 0.055,
                };
                (
                    Vec2::new(x, cy + ch * 0.16),
                    Vec2::new(10.0, (ch * 0.40).clamp(50.0, 150.0)),
                )
            }
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

    for zone in 0..3 {
        commands.spawn((
            Mesh2d(quad.clone()),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(color::SURFACE_RAISED))),
            Transform::default(),
            Panel(zone),
        ));
    }

    for widget in all_widgets() {
        commands.spawn((
            Mesh2d(quad.clone()),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(color::WIDGET_BG))),
            Transform::default(),
            widget,
        ));
        let label_color = match widget {
            Widget::Button(_) => color::TEXT_PRIMARY,
            Widget::Slider(_) => color::TEXT_MUTED,
        };
        commands.spawn((
            Text2d::new(label_text(widget)),
            TextFont {
                font: fonts.text.clone().into(),
                font_size: FontSize::Px(font::CAPTION),
                ..Default::default()
            },
            TextColor(label_color),
            Anchor::CENTER,
            Transform::default(),
            Label(widget),
        ));
        if let Widget::Slider(kind) = widget {
            commands.spawn((
                Mesh2d(quad.clone()),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(kind.accent()))),
                Transform::default(),
                Fill(kind),
            ));
            commands.spawn((
                Mesh2d(quad.clone()),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(color::THUMB))),
                Transform::default(),
                Thumb(kind),
            ));
        }
    }
}

/// Positionne panneaux, pistes, jauges, curseurs et étiquettes depuis
/// `slot` (recalcul par frame : quelques multiplications, aucune géométrie
/// touchée).
#[allow(clippy::type_complexity)] // filtres de requêtes Bevy disjointes
fn place_widgets(
    windows: Query<&Window>,
    mix: Res<MixState>,
    mut sets: ParamSet<(
        Query<(&mut Transform, &Widget)>,
        Query<(&mut Transform, &Fill)>,
        Query<(&mut Transform, &Thumb)>,
        Query<(&mut Transform, &Label)>,
        Query<(&mut Transform, &Panel)>,
    )>,
) {
    let Ok(window) = windows.single() else { return };
    let window = window.clone();
    let (w, h) = (window.width(), window.height());
    let bands = layout::bands(w, h);

    for (mut transform, panel) in &mut sets.p4() {
        let (x, width) = match panel.0 {
            0 => deck_zone(0, w),
            1 => deck_zone(1, w),
            _ => (0.0, w * 0.36),
        };
        transform.translation = Vec3::new(x, bands.controls_center, 2.0);
        transform.scale = Vec3::new(width + layout::GAP * 2.0, bands.controls_height, 1.0);
    }

    for (mut transform, widget) in &mut sets.p0() {
        let (center, size) = slot(*widget, &window);
        transform.translation = center.extend(3.0);
        transform.scale = Vec3::new(size.x, size.y, 1.0);
    }

    for (mut transform, fill) in &mut sets.p1() {
        let (center, size) = slot(Widget::Slider(fill.0), &window);
        let t = slider_value(fill.0, &mix);
        let (offset, fill_size) = if fill.0.horizontal() {
            if fill.0.bipolar() {
                let extent = (t - 0.5) * size.x;
                (
                    Vec2::new(extent * 0.5, 0.0),
                    Vec2::new(extent.abs(), size.y),
                )
            } else {
                let extent = t * size.x;
                (
                    Vec2::new((extent - size.x) * 0.5, 0.0),
                    Vec2::new(extent, size.y),
                )
            }
        } else if fill.0.bipolar() {
            let extent = (t - 0.5) * size.y;
            (
                Vec2::new(0.0, extent * 0.5),
                Vec2::new(size.x, extent.abs()),
            )
        } else {
            let extent = t * size.y;
            (
                Vec2::new(0.0, (extent - size.y) * 0.5),
                Vec2::new(size.x, extent),
            )
        };
        transform.translation = (center + offset).extend(3.5);
        transform.scale = Vec3::new(fill_size.x.max(0.5), fill_size.y.max(0.5), 1.0);
    }

    for (mut transform, thumb) in &mut sets.p2() {
        let (center, size) = slot(Widget::Slider(thumb.0), &window);
        let t = slider_value(thumb.0, &mix) - 0.5;
        let (offset, thumb_size) = if thumb.0.horizontal() {
            (Vec2::new(t * size.x, 0.0), Vec2::new(6.0, size.y + 10.0))
        } else {
            (Vec2::new(0.0, t * size.y), Vec2::new(size.x + 10.0, 6.0))
        };
        transform.translation = (center + offset).extend(4.0);
        transform.scale = Vec3::new(thumb_size.x, thumb_size.y, 1.0);
    }

    for (mut transform, label) in &mut sets.p3() {
        let (center, size) = slot(label.0, &window);
        let position = match label.0 {
            Widget::Button(_) => center,
            Widget::Slider(_) => center - Vec2::new(0.0, size.y * 0.5 + 11.0),
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
    mut browser: ResMut<Browser>,
    browser_view: Res<crate::browser::BrowserView>,
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
    // Les clics dans la bibliothèque ouverte lui appartiennent.
    if browser.open && browser_view.contains(point) && mouse.just_pressed(MouseButton::Left) {
        return;
    }
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
                    ButtonKind::Load(_) => browser.open = true,
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
