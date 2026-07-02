//! Bibliothèque intégrée en **Bevy natif** (quads + Text2d du design
//! system) : panneau latéral droit listant dossiers et fichiers audio,
//! pilotable aux trois entrées — mêmes intentions partout (specs §6.4) :
//!
//! - **contrôleur** : encodeur BROWSER (`LibraryScroll`), poussoir
//!   (`LibraryEnter` : entre dans le dossier), boutons Load (charge la
//!   sélection sur le deck ; bibliothèque fermée : l'ouvre) ;
//! - **clavier** (modal quand ouverte, le contrôleur reste actif) :
//!   `↑`/`↓` sélection, `→`/`Entrée` entrer, `←` dossier parent,
//!   `F`/`L` charger la sélection sur A/B, `B`/`Échap` fermer ;
//! - **souris** : clic = sélectionner, re-clic = entrer (dossier),
//!   molette = défiler, boutons « → A »/« → B » = charger.
//!
//! Une ligne « .. » synthétique en tête permet la remontée au parent avec
//! le seul encodeur. Le dialogue système `rfd` reste accessible depuis le
//! panneau F12.

use std::path::PathBuf;

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::sprite::Anchor;

use engine::Deck;

use crate::fonts::UiFonts;
use crate::theme::{color, font, layout};
use crate::{LoadSender, spawn_load_worker};

const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "flac", "wav", "ogg", "m4a", "aac"];
/// Pre-created row pool — sized for tall windows so the list always fills
/// the panel down to the metadata strip (extra rows stay hidden).
const MAX_ROWS: usize = 64;
const ROW_HEIGHT: f32 = 24.0;
/// Bottom strip heights (from the bottom edge): hint line, load buttons,
/// then the 2-line metadata block; the file list stops right above it.
const META_TOP: f32 = 84.0;
const META_BASELINE: f32 = 46.0;

pub struct BrowserPlugin;

impl Plugin for BrowserPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Browser::default())
            .init_resource::<BrowserView>()
            .add_systems(Startup, spawn_browser)
            .add_systems(Update, (keys_input, mouse_input, render, place).chain());
    }
}

pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Resource)]
pub struct Browser {
    pub open: bool,
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    scroll: usize,
    dirty: bool,
    /// Metadata strip content, rebuilt when the selection changes
    /// (`meta_index` is the entry it was computed for).
    meta_text: String,
    meta_index: Option<usize>,
}

impl Default for Browser {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        // La bibliothèque de l'utilisateur si elle existe, sinon le home.
        let dir = [home.join("Musique"), home.join("Music")]
            .into_iter()
            .find(|p| p.is_dir())
            .unwrap_or(home);
        Self {
            open: true,
            dir,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            dirty: true,
            meta_text: String::new(),
            meta_index: None,
        }
    }
}

impl Browser {
    /// Déplace la sélection (encodeur, flèches, molette).
    pub fn scroll_by(&mut self, delta: i32) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        self.selected = self
            .selected
            .saturating_add_signed(delta as isize)
            .min(last);
    }

    /// Entre dans le dossier sélectionné (poussoir de l'encodeur, `→`).
    pub fn enter(&mut self) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };
        if entry.is_dir {
            self.navigate(entry.path.clone());
        }
    }

    pub fn go_parent(&mut self) {
        if let Some(parent) = self.dir.parent() {
            self.navigate(parent.to_path_buf());
        }
    }

    /// Charge la piste sélectionnée sur `deck` (boutons Load du contrôleur,
    /// touches `F`/`L`, boutons souris).
    pub fn load_selected(&self, deck: Deck, tx: &LoadSender) {
        if let Some(entry) = self.entries.get(self.selected)
            && !entry.is_dir
        {
            spawn_load_worker(entry.path.clone(), deck, tx.tx.clone(), tx.sample_rate);
        }
    }

    fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.selected = 0;
        self.scroll = 0;
        self.dirty = true;
    }

    /// Rebuilds the metadata strip for the selected entry when it changed.
    /// The audio probe reads headers only (`decode::probe_info`) — cheap,
    /// and it runs at selection speed, not per frame.
    fn refresh_metadata(&mut self) {
        if self.meta_index == Some(self.selected) {
            return;
        }
        self.meta_index = Some(self.selected);
        self.meta_text = match self.entries.get(self.selected) {
            None => String::new(),
            Some(entry) if entry.is_dir => "dossier".into(),
            Some(entry) => {
                let size = std::fs::metadata(&entry.path)
                    .map(|m| format!("{:.1} Mo", m.len() as f64 / 1_048_576.0))
                    .unwrap_or_else(|_| "?".into());
                let ext = entry
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_uppercase)
                    .unwrap_or_default();
                let info = decode::probe_info(&entry.path).unwrap_or_default();
                let mut tech = format!("{size} · {ext}");
                if let Some(seconds) = info.duration_seconds {
                    let s = seconds.round() as u64;
                    tech.push_str(&format!(" · {}:{:02}", s / 60, s % 60));
                }
                if let Some(rate) = info.sample_rate {
                    tech.push_str(&format!(" · {:.1} kHz", f64::from(rate) / 1000.0));
                }
                if let Some(channels) = info.channels {
                    tech.push_str(&format!(" · {channels} can."));
                }
                match (info.artist, info.title) {
                    (Some(artist), Some(title)) => format!("{tech}\n{artist} — {title}"),
                    (Some(artist), None) => format!("{tech}\n{artist}"),
                    (None, Some(title)) => format!("{tech}\n{title}"),
                    (None, None) => tech,
                }
            }
        };
    }

    fn refresh(&mut self) {
        self.dirty = false;
        self.meta_index = None;
        self.entries.clear();
        // Ligne « .. » synthétique : la remontée au parent reste possible
        // avec le seul encodeur du contrôleur.
        if let Some(parent) = self.dir.parent() {
            self.entries.push(Entry {
                name: "..".into(),
                path: parent.to_path_buf(),
                is_dir: true,
            });
        }
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut listed: Vec<Entry> = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().display().to_string();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = path.is_dir();
            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
            if is_dir || is_audio {
                listed.push(Entry { name, path, is_dir });
            }
        }
        listed.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.entries.extend(listed);
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }
}

/// Géométrie du panneau, partagée avec les autres systèmes d'entrée
/// (les clics/molette dans cette zone ne vont ni aux widgets ni au zoom).
#[derive(Resource, Default, Clone, Copy)]
pub struct BrowserView {
    pub rect_center: Vec2,
    pub rect_size: Vec2,
    rows_visible: usize,
}

impl BrowserView {
    pub fn contains(&self, point: Vec2) -> bool {
        (point - self.rect_center)
            .abs()
            .cmple(self.rect_size * 0.5)
            .all()
    }
}

#[derive(Component)]
struct Backdrop;
#[derive(Component)]
struct TitleText;
#[derive(Component)]
struct HintText;
#[derive(Component)]
struct MetaText;
#[derive(Component)]
struct Row(usize);
#[derive(Component)]
struct RowHighlight;
#[derive(Component)]
struct LoadButton(usize);
#[derive(Component)]
struct LoadButtonLabel(usize);

fn spawn_browser(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fonts: Res<UiFonts>,
) {
    let quad = meshes.add(Rectangle::new(1.0, 1.0));

    commands.spawn((
        Mesh2d(quad.clone()),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(color::SURFACE_RAISED))),
        Transform::default(),
        Backdrop,
    ));
    commands.spawn((
        Mesh2d(quad.clone()),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(color::WIDGET_BG))),
        Transform::default(),
        RowHighlight,
    ));
    commands.spawn((
        Text2d::new("Bibliothèque"),
        TextFont {
            font: fonts.text.clone().into(),
            font_size: FontSize::Px(font::BODY),
            ..Default::default()
        },
        TextColor(color::TEXT_PRIMARY),
        Anchor::TOP_LEFT,
        Transform::default(),
        TitleText,
    ));
    commands.spawn((
        Text2d::new("↑↓ naviguer   → entrer   F/L → deck   B fermer"),
        TextFont {
            font: fonts.text.clone().into(),
            font_size: FontSize::Px(font::CAPTION),
            ..Default::default()
        },
        TextColor(color::TEXT_MUTED),
        Anchor::BOTTOM_LEFT,
        Transform::default(),
        HintText,
    ));
    commands.spawn((
        Text2d::new(""),
        TextFont {
            font: fonts.text.clone().into(),
            font_size: FontSize::Px(font::CAPTION),
            ..Default::default()
        },
        TextColor(color::TEXT_MUTED),
        Anchor::BOTTOM_LEFT,
        Transform::default(),
        MetaText,
    ));
    for index in 0..MAX_ROWS {
        commands.spawn((
            Text2d::new(""),
            TextFont {
                font: fonts.text.clone().into(),
                font_size: FontSize::Px(font::CAPTION),
                ..Default::default()
            },
            TextColor(color::TEXT_MUTED),
            Anchor::CENTER_LEFT,
            Transform::default(),
            Row(index),
        ));
    }
    for deck in 0..2 {
        let accent = if deck == 0 {
            color::DECK_A
        } else {
            color::DECK_B
        };
        commands.spawn((
            Mesh2d(quad.clone()),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(color::WIDGET_BG))),
            Transform::default(),
            LoadButton(deck),
        ));
        commands.spawn((
            Text2d::new(if deck == 0 { "→ A" } else { "→ B" }),
            TextFont {
                font: fonts.text.clone().into(),
                font_size: FontSize::Px(font::CAPTION),
                ..Default::default()
            },
            TextColor(accent),
            Anchor::CENTER,
            Transform::default(),
            LoadButtonLabel(deck),
        ));
    }
}

/// Clavier — modal quand la bibliothèque est ouverte (le contrôleur MIDI,
/// lui, reste pleinement actif sur les decks).
fn keys_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut browser: ResMut<Browser>,
    load_tx: Res<LoadSender>,
) {
    if keys.just_pressed(KeyCode::KeyB) {
        browser.open = !browser.open;
        return;
    }
    if !browser.open {
        if keys.just_pressed(KeyCode::KeyF) || keys.just_pressed(KeyCode::KeyL) {
            browser.open = true;
        }
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        browser.open = false;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        browser.scroll_by(-1);
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        browser.scroll_by(1);
    }
    if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::Enter) {
        browser.enter();
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        browser.go_parent();
    }
    if keys.just_pressed(KeyCode::KeyF) {
        browser.load_selected(Deck::A, &load_tx);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        browser.load_selected(Deck::B, &load_tx);
    }
}

fn mouse_input(
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    view: Res<BrowserView>,
    mut browser: ResMut<Browser>,
    load_buttons: Query<(&Transform, &LoadButton)>,
    load_tx: Res<LoadSender>,
) {
    if !browser.open {
        wheel.clear();
        return;
    }
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let point = Vec2::new(
        cursor.x - window.width() * 0.5,
        window.height() * 0.5 - cursor.y,
    );
    if !view.contains(point) {
        wheel.clear();
        return;
    }

    for event in wheel.read() {
        browser.scroll_by(if event.y > 0.0 { -3 } else { 3 });
    }

    if mouse.just_pressed(MouseButton::Left) {
        for (transform, button) in &load_buttons {
            let half = transform.scale.truncate() * 0.5;
            if (point - transform.translation.truncate())
                .abs()
                .cmple(half)
                .all()
            {
                let deck = if button.0 == 0 { Deck::A } else { Deck::B };
                browser.load_selected(deck, &load_tx);
                return;
            }
        }
        // Clic sur une ligne : sélectionne ; re-clic : entre (dossier).
        let list_top = view.rect_center.y + view.rect_size.y * 0.5 - 56.0;
        if point.y <= list_top {
            let index = ((list_top - point.y) / ROW_HEIGHT) as usize;
            let target = browser.scroll + index;
            if target < browser.entries.len() {
                if browser.selected == target {
                    browser.enter();
                } else {
                    browser.selected = target;
                }
            }
        }
    }
}

/// Met à jour visibilités et contenus (uniquement quand l'état change).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn render(
    mut browser: ResMut<Browser>,
    view: Res<BrowserView>,
    mut rows: Query<(&mut Text2d, &mut TextColor, &mut Visibility, &Row)>,
    mut title: Query<
        (&mut Text2d, &mut Visibility),
        (With<TitleText>, Without<Row>, Without<HintText>),
    >,
    mut hints: Query<
        &mut Visibility,
        (
            With<HintText>,
            Without<Row>,
            Without<TitleText>,
            Without<Backdrop>,
            Without<MetaText>,
        ),
    >,
    mut meta: Query<
        (&mut Text2d, &mut Visibility),
        (
            With<MetaText>,
            Without<Row>,
            Without<TitleText>,
            Without<HintText>,
        ),
    >,
    mut chrome: Query<
        &mut Visibility,
        (
            Or<(
                With<Backdrop>,
                With<RowHighlight>,
                With<LoadButton>,
                With<LoadButtonLabel>,
            )>,
            Without<Row>,
            Without<TitleText>,
            Without<HintText>,
            Without<MetaText>,
        ),
    >,
) {
    if browser.dirty {
        browser.refresh();
    }
    browser.refresh_metadata();
    // Fenêtre de défilement autour de la sélection.
    let visible = view.rows_visible.max(1);
    if browser.selected < browser.scroll {
        browser.scroll = browser.selected;
    } else if browser.selected >= browser.scroll + visible {
        browser.scroll = browser.selected + 1 - visible;
    }

    let shown = if browser.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut chrome {
        *visibility = shown;
    }
    for mut visibility in &mut hints {
        *visibility = shown;
    }
    if let Ok((mut text, mut visibility)) = title.single_mut() {
        *visibility = shown;
        if browser.open {
            text.0 = format!(
                "Bibliothèque — {}  ({}/{})",
                browser.dir.display(),
                (browser.selected + 1).min(browser.entries.len()),
                browser.entries.len()
            );
        }
    }
    if let Ok((mut text, mut visibility)) = meta.single_mut() {
        *visibility = shown;
        if browser.open && text.0 != browser.meta_text {
            text.0.clone_from(&browser.meta_text);
        }
    }

    for (mut text, mut text_color, mut visibility, row) in &mut rows {
        let index = browser.scroll + row.0;
        let entry = browser.entries.get(index);
        let visible_row = browser.open && row.0 < visible && entry.is_some();
        *visibility = if visible_row {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(entry) = entry
            && visible_row
        {
            let icon = if entry.is_dir { "▸" } else { "♪" };
            text.0 = format!("{icon} {}", entry.name);
            text_color.0 = if index == browser.selected {
                color::TEXT_PRIMARY
            } else {
                color::TEXT_MUTED
            };
        }
    }
}

/// Géométrie du panneau (fractions de fenêtre) et placement des entités.
#[allow(clippy::type_complexity)]
fn place(
    windows: Query<&Window>,
    browser: Res<Browser>,
    mut view: ResMut<BrowserView>,
    mut sets: ParamSet<(
        Query<&mut Transform, With<Backdrop>>,
        Query<&mut Transform, With<TitleText>>,
        Query<&mut Transform, With<HintText>>,
        Query<(&mut Transform, &Row)>,
        Query<&mut Transform, With<RowHighlight>>,
        Query<(&mut Transform, &LoadButton)>,
        Query<(&mut Transform, &LoadButtonLabel)>,
        Query<&mut Transform, With<MetaText>>,
    )>,
) {
    let Ok(window) = windows.single() else { return };
    let (w, h) = (window.width(), window.height());
    let width = (w * 0.30).clamp(300.0, 500.0).min(w - 2.0 * layout::MARGIN);
    let height = h - 2.0 * layout::MARGIN;
    let center = Vec2::new(w * 0.5 - width * 0.5 - layout::MARGIN, 0.0);

    let list_top = height * 0.5 - 56.0;
    let footer = -height * 0.5 + 40.0;
    view.rect_center = center;
    view.rect_size = Vec2::new(width, height);
    // The list fills the panel down to the metadata strip whatever the
    // window height (the row pool is sized accordingly).
    let list_bottom = -height * 0.5 + META_TOP;
    view.rows_visible = (((list_top - list_bottom) / ROW_HEIGHT) as usize).min(MAX_ROWS);

    if let Ok(mut transform) = sets.p0().single_mut() {
        transform.translation = center.extend(10.0);
        transform.scale = Vec3::new(width, height, 1.0);
    }
    if let Ok(mut transform) = sets.p1().single_mut() {
        transform.translation = Vec3::new(center.x - width * 0.5 + 12.0, height * 0.5 - 10.0, 11.0);
    }
    if let Ok(mut transform) = sets.p2().single_mut() {
        transform.translation = Vec3::new(center.x - width * 0.5 + 12.0, -height * 0.5 + 8.0, 11.0);
    }
    for (mut transform, row) in &mut sets.p3() {
        let y = list_top - (row.0 as f32 + 0.5) * ROW_HEIGHT;
        transform.translation = Vec3::new(center.x - width * 0.5 + 14.0, y, 11.0);
    }
    // Surbrillance de la ligne sélectionnée.
    let selected_offset = browser.selected.saturating_sub(browser.scroll) as f32;
    if let Ok(mut transform) = sets.p4().single_mut() {
        let y = list_top - (selected_offset + 0.5) * ROW_HEIGHT;
        transform.translation = Vec3::new(center.x, y, 10.5);
        transform.scale = Vec3::new(width - 12.0, ROW_HEIGHT - 2.0, 1.0);
    }
    for (mut transform, button) in &mut sets.p5() {
        let x = center.x + (button.0 as f32 - 0.5) * 96.0;
        transform.translation = Vec3::new(x, footer - 12.0, 11.0);
        transform.scale = Vec3::new(84.0, 26.0, 1.0);
    }
    for (mut transform, label) in &mut sets.p6() {
        let x = center.x + (label.0 as f32 - 0.5) * 96.0;
        transform.translation = Vec3::new(x, footer - 12.0, 12.0);
    }
    if let Ok(mut transform) = sets.p7().single_mut() {
        transform.translation = Vec3::new(
            center.x - width * 0.5 + 12.0,
            -height * 0.5 + META_BASELINE,
            11.0,
        );
    }
}
