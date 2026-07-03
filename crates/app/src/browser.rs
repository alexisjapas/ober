//! Integrated library in **native Bevy** (design-system quads + Text2d):
//! a **permanent band** of the single screen (`theme::layout::bands()`) —
//! same layer as the waveforms and controls, always visible, never an
//! overlay. Two panes: folders on the left, audio files of the *selected*
//! folder on the right with metadata columns (title, artist, BPM,
//! duration — tags read by a background header probe,
//! `decode::probe_info`, never on the UI thread).
//!
//! Same intents on every input (specs §6.4):
//!
//! - **controller**: BROWSER encoder = file list, Shift + encoder = folder
//!   list (selecting a folder instantly previews its files), push = enter
//!   the selected folder, Load buttons = load the selection;
//! - **keyboard**: `B` toggles the keyboard *focus* (focused, the deck
//!   shortcuts pause): `↑`/`↓` files, `Shift+↑`/`↓` folders, `Entrée`/`→`
//!   enter, `←`/`Retour` parent, `Échap` unfocus. `F`/`L` load onto A/B
//!   whatever the focus;
//! - **mouse**: click = select and focus (folder pane: re-click = enter),
//!   wheel over a pane = scroll it, « → A »/« → B » buttons = load.
//!
//! The folder list starts with a synthetic ".." row (parent) then the
//! current folder itself — everything stays reachable with the encoder
//! alone. The `rfd` system dialog remains available from the F12 panel.

use std::collections::HashMap;
use std::path::PathBuf;

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::sprite::Anchor;

use engine::Deck;

use crate::fonts::UiFonts;
use crate::theme::{color, font, layout};
use crate::{LoadSender, spawn_load_worker};

const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "flac", "wav", "ogg", "m4a", "aac"];
/// Pre-created row pools (extras stay hidden) — sized for tall windows.
const MAX_DIR_ROWS: usize = 32;
const MAX_FILE_ROWS: usize = 32;
const ROW_HEIGHT: f32 = 24.0;
/// File-pane column offsets, as fractions of the pane width.
const COLUMNS: [(&str, f32); 4] = [
    ("Titre", 0.0),
    ("Artiste", 0.50),
    ("BPM", 0.78),
    ("Durée", 0.88),
];

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
}

#[derive(Resource)]
pub struct Browser {
    /// Keyboard focus only — the panel itself is always visible. While
    /// focused, the deck keyboard shortcuts pause (`B` toggles).
    pub focused: bool,
    /// Current directory: its children fill the folder pane.
    dir: PathBuf,
    /// Folder pane rows: "..", the current folder itself, then subfolders.
    dirs: Vec<Entry>,
    /// File pane rows: audio files of the selected folder-pane entry.
    files: Vec<Entry>,
    dir_selected: usize,
    file_selected: usize,
    dir_scroll: usize,
    file_scroll: usize,
    dirty: bool,
    files_dirty: bool,
    /// Header-probe cache; filled asynchronously by the probe worker.
    meta: HashMap<PathBuf, decode::ProbeInfo>,
    probe_tx: crossbeam_channel::Sender<Vec<PathBuf>>,
    probe_results: crossbeam_channel::Receiver<(PathBuf, decode::ProbeInfo)>,
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
        let (probe_tx, probe_results) = spawn_probe_worker();
        Self {
            focused: false,
            dir,
            dirs: Vec::new(),
            files: Vec::new(),
            dir_selected: 0,
            file_selected: 0,
            dir_scroll: 0,
            file_scroll: 0,
            dirty: true,
            files_dirty: true,
            meta: HashMap::new(),
            probe_tx,
            probe_results,
        }
    }
}

/// Long-lived metadata worker: receives batches of paths, probes headers
/// off the UI thread, streams results back. Scroll spam coalesces to the
/// latest batch; the worker never probes the same path twice.
fn spawn_probe_worker() -> (
    crossbeam_channel::Sender<Vec<PathBuf>>,
    crossbeam_channel::Receiver<(PathBuf, decode::ProbeInfo)>,
) {
    let (req_tx, req_rx) = crossbeam_channel::unbounded::<Vec<PathBuf>>();
    let (res_tx, res_rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("browser-probe".into())
        .spawn(move || {
            let mut probed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
            while let Ok(mut batch) = req_rx.recv() {
                while let Ok(newer) = req_rx.try_recv() {
                    batch = newer;
                }
                for path in batch {
                    if !probed.insert(path.clone()) {
                        continue;
                    }
                    let info = decode::probe_info(&path).unwrap_or_default();
                    if res_tx.send((path, info)).is_err() {
                        return;
                    }
                }
            }
        })
        .ok();
    (req_tx, res_rx)
}

impl Browser {
    /// Moves the file-list selection (encoder, arrows, wheel). Empty file
    /// list (folder without direct audio files — e.g. a library nested per
    /// album): fall back to the folder pane, so the primary encoder motion
    /// walks down the tree instead of spinning on nothing.
    pub fn scroll_by(&mut self, delta: i32) {
        if self.files.is_empty() {
            self.scroll_dirs_by(delta);
            return;
        }
        let last = self.files.len() - 1;
        self.file_selected = self
            .file_selected
            .saturating_add_signed(delta as isize)
            .min(last);
    }

    /// Moves the folder-pane selection (Shift + encoder, Shift + arrows):
    /// the file pane instantly previews the newly selected folder.
    pub fn scroll_dirs_by(&mut self, delta: i32) {
        if self.dirs.is_empty() {
            return;
        }
        let last = self.dirs.len() - 1;
        let next = self
            .dir_selected
            .saturating_add_signed(delta as isize)
            .min(last);
        if next != self.dir_selected {
            self.dir_selected = next;
            self.files_dirty = true;
        }
    }

    /// Entre dans le dossier sélectionné (poussoir de l'encodeur, `→`).
    pub fn enter(&mut self) {
        let Some(entry) = self.dirs.get(self.dir_selected) else {
            return;
        };
        if entry.path != self.dir {
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
        if let Some(entry) = self.files.get(self.file_selected) {
            spawn_load_worker(entry.path.clone(), deck, tx.tx.clone(), tx.sample_rate);
        }
    }

    fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.dirty = true;
    }

    /// Lists the folder pane: "..", the current folder, its subfolders.
    fn refresh(&mut self) {
        self.dirty = false;
        self.dirs.clear();
        if let Some(parent) = self.dir.parent() {
            self.dirs.push(Entry {
                name: "..".into(),
                path: parent.to_path_buf(),
            });
        }
        let current = self
            .dir
            .file_name()
            .map(|n| n.display().to_string())
            .unwrap_or_else(|| self.dir.display().to_string());
        // The current folder row: selected by default, so entering a
        // folder immediately lists its own files in the right pane.
        self.dir_selected = self.dirs.len();
        self.dirs.push(Entry {
            name: current,
            path: self.dir.clone(),
        });

        let mut listed: Vec<Entry> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.dir) {
            for entry in read.flatten() {
                let path = entry.path();
                let name = entry.file_name().display().to_string();
                if name.starts_with('.') || !path.is_dir() {
                    continue;
                }
                listed.push(Entry { name, path });
            }
        }
        listed.sort_by_key(|e| e.name.to_lowercase());
        self.dirs.extend(listed);
        self.dir_scroll = 0;
        self.files_dirty = true;
    }

    /// Lists the file pane (audio files of the selected folder) and asks
    /// the worker for the metadata still missing from the cache.
    fn refresh_files(&mut self) {
        self.files_dirty = false;
        self.files.clear();
        self.file_selected = 0;
        self.file_scroll = 0;
        let Some(selected) = self.dirs.get(self.dir_selected) else {
            return;
        };
        if let Ok(read) = std::fs::read_dir(&selected.path) {
            for entry in read.flatten() {
                let path = entry.path();
                let name = entry.file_name().display().to_string();
                if name.starts_with('.') || path.is_dir() {
                    continue;
                }
                let is_audio = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()));
                if is_audio {
                    self.files.push(Entry { name, path });
                }
            }
        }
        self.files.sort_by_key(|e| e.name.to_lowercase());

        let missing: Vec<PathBuf> = self
            .files
            .iter()
            .map(|f| f.path.clone())
            .filter(|p| !self.meta.contains_key(p))
            .collect();
        if !missing.is_empty() {
            let _ = self.probe_tx.send(missing);
        }
    }
}

/// Géométrie du panneau, partagée avec les autres systèmes d'entrée
/// (les clics/molette dans cette zone ne vont ni aux widgets ni au zoom).
#[derive(Resource, Default, Clone, Copy)]
pub struct BrowserView {
    pub rect_center: Vec2,
    pub rect_size: Vec2,
    /// Width of the folder pane (mouse pane hit-testing).
    left_width: f32,
    rows_visible: usize,
}

impl BrowserView {
    pub fn contains(&self, point: Vec2) -> bool {
        (point - self.rect_center)
            .abs()
            .cmple(self.rect_size * 0.5)
            .all()
    }

    /// Top of the row area, in screen coordinates.
    fn list_top(&self) -> f32 {
        self.rect_center.y + self.rect_size.y * 0.5 - 56.0
    }

    /// X of the folder/file pane split, in screen coordinates.
    fn split_x(&self) -> f32 {
        self.rect_center.x - self.rect_size.x * 0.5 + self.left_width
    }
}

/// Every panel entity carries this single marker: one query per system,
/// no `Without` chains (placement, visibility and text all match on it).
#[derive(Component, Clone, Copy, PartialEq)]
enum Part {
    Backdrop,
    Divider,
    Title,
    Hint,
    DirHeader,
    FileHeader(usize),
    DirRow(usize),
    FileCell { row: usize, col: usize },
    DirHighlight,
    FileHighlight,
    LoadButton(usize),
    LoadLabel(usize),
}

fn spawn_browser(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fonts: Res<UiFonts>,
) {
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let mut quad_part = |part: Part, color: Color| {
        commands.spawn((
            Mesh2d(quad.clone()),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(color))),
            Transform::default(),
            part,
        ));
    };
    quad_part(Part::Backdrop, color::SURFACE_RAISED);
    quad_part(Part::Divider, color::WIDGET_BG);
    quad_part(Part::DirHighlight, color::WIDGET_BG);
    quad_part(Part::FileHighlight, color::WIDGET_BG);
    quad_part(Part::LoadButton(0), color::WIDGET_BG);
    quad_part(Part::LoadButton(1), color::WIDGET_BG);

    let text_part = |commands: &mut Commands,
                     part: Part,
                     text: &str,
                     size: f32,
                     text_color: Color,
                     anchor: Anchor| {
        commands.spawn((
            Text2d::new(text),
            TextFont {
                font: fonts.text.clone().into(),
                font_size: FontSize::Px(size),
                ..Default::default()
            },
            TextColor(text_color),
            anchor,
            Transform::default(),
            part,
        ));
    };

    text_part(
        &mut commands,
        Part::Title,
        "Bibliothèque",
        font::BODY,
        color::TEXT_PRIMARY,
        Anchor::TOP_LEFT,
    );
    text_part(
        &mut commands,
        Part::Hint,
        "↑↓ fichiers   Maj+↑↓ dossiers   Entrée entrer   ← parent   F/L → deck   B fermer",
        font::CAPTION,
        color::TEXT_MUTED,
        Anchor::BOTTOM_LEFT,
    );
    text_part(
        &mut commands,
        Part::DirHeader,
        "Dossiers",
        font::CAPTION,
        color::TEXT_MUTED,
        Anchor::CENTER_LEFT,
    );
    for (i, (label, _)) in COLUMNS.iter().enumerate() {
        text_part(
            &mut commands,
            Part::FileHeader(i),
            label,
            font::CAPTION,
            color::TEXT_MUTED,
            Anchor::CENTER_LEFT,
        );
    }
    for index in 0..MAX_DIR_ROWS {
        text_part(
            &mut commands,
            Part::DirRow(index),
            "",
            font::CAPTION,
            color::TEXT_MUTED,
            Anchor::CENTER_LEFT,
        );
    }
    for row in 0..MAX_FILE_ROWS {
        for col in 0..COLUMNS.len() {
            text_part(
                &mut commands,
                Part::FileCell { row, col },
                "",
                font::CAPTION,
                color::TEXT_MUTED,
                Anchor::CENTER_LEFT,
            );
        }
    }
    for deck in 0..2 {
        let accent = if deck == 0 {
            color::DECK_A
        } else {
            color::DECK_B
        };
        text_part(
            &mut commands,
            Part::LoadLabel(deck),
            if deck == 0 { "→ A" } else { "→ B" },
            font::CAPTION,
            accent,
            Anchor::CENTER,
        );
    }
}

/// Clavier. `B` bascule le focus (la bibliothèque est toujours visible ;
/// focalisée, les raccourcis decks sont en pause) ; `F`/`L` chargent la
/// sélection quel que soit le focus. Même sémantique que l'encodeur :
/// base = fichiers, Shift = dossiers, Entrée = entrer.
fn keys_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut browser: ResMut<Browser>,
    load_tx: Res<LoadSender>,
) {
    if keys.just_pressed(KeyCode::KeyB) {
        browser.focused = !browser.focused;
        return;
    }
    if keys.just_pressed(KeyCode::KeyF) {
        browser.load_selected(Deck::A, &load_tx);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        browser.load_selected(Deck::B, &load_tx);
    }
    if !browser.focused {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        browser.focused = false;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if keys.just_pressed(KeyCode::ArrowUp) {
        if shift {
            browser.scroll_dirs_by(-1);
        } else {
            browser.scroll_by(-1);
        }
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        if shift {
            browser.scroll_dirs_by(1);
        } else {
            browser.scroll_by(1);
        }
    }
    if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::Enter) {
        browser.enter();
    }
    if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::Backspace) {
        browser.go_parent();
    }
}

fn mouse_input(
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    view: Res<BrowserView>,
    mut browser: ResMut<Browser>,
    parts: Query<(&Transform, &Part)>,
    load_tx: Res<LoadSender>,
) {
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
    if mouse.just_pressed(MouseButton::Left) {
        // Un clic dans la bibliothèque lui donne aussi le focus clavier.
        browser.focused = true;
    }

    let in_folder_pane = point.x < view.split_x();
    for event in wheel.read() {
        let delta = if event.y > 0.0 { -3 } else { 3 };
        if in_folder_pane {
            browser.scroll_dirs_by(delta);
        } else {
            browser.scroll_by(delta);
        }
    }

    if mouse.just_pressed(MouseButton::Left) {
        for (transform, part) in &parts {
            let Part::LoadButton(deck) = part else {
                continue;
            };
            let half = transform.scale.truncate() * 0.5;
            if (point - transform.translation.truncate())
                .abs()
                .cmple(half)
                .all()
            {
                let deck = if *deck == 0 { Deck::A } else { Deck::B };
                browser.load_selected(deck, &load_tx);
                return;
            }
        }
        let list_top = view.list_top();
        if point.y <= list_top {
            let index = ((list_top - point.y) / ROW_HEIGHT) as usize;
            if in_folder_pane {
                let target = browser.dir_scroll + index;
                if target < browser.dirs.len() {
                    if browser.dir_selected == target {
                        browser.enter();
                    } else {
                        browser.dir_selected = target;
                        browser.files_dirty = true;
                    }
                }
            } else {
                let target = browser.file_scroll + index;
                if target < browser.files.len() {
                    browser.file_selected = target;
                }
            }
        }
    }
}

fn format_duration(seconds: f64) -> String {
    let s = seconds.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Met à jour visibilités et contenus (état + cache de métadonnées).
fn render(
    mut browser: ResMut<Browser>,
    view: Res<BrowserView>,
    mut texts: Query<(&mut Text2d, &mut TextColor, &mut Visibility, &Part)>,
    mut chrome: Query<(&mut Visibility, &Part), Without<Text2d>>,
) {
    let browser = &mut *browser;
    while let Ok((path, info)) = browser.probe_results.try_recv() {
        browser.meta.insert(path, info);
    }
    if browser.dirty {
        browser.refresh();
    }
    if browser.files_dirty {
        browser.refresh_files();
    }

    // Fenêtres de défilement autour des sélections.
    let visible = view.rows_visible.max(1);
    if browser.dir_selected < browser.dir_scroll {
        browser.dir_scroll = browser.dir_selected;
    } else if browser.dir_selected >= browser.dir_scroll + visible {
        browser.dir_scroll = browser.dir_selected + 1 - visible;
    }
    if browser.file_selected < browser.file_scroll {
        browser.file_scroll = browser.file_selected;
    } else if browser.file_selected >= browser.file_scroll + visible {
        browser.file_scroll = browser.file_selected + 1 - visible;
    }

    // Toujours visible (bande permanente du layout) : seules les lignes
    // au-delà des listes sont masquées.
    for (mut visibility, _) in &mut chrome {
        *visibility = Visibility::Inherited;
    }

    for (mut text, mut text_color, mut visibility, part) in &mut texts {
        match *part {
            Part::Title => {
                *visibility = Visibility::Inherited;
                text.0 = format!(
                    "Bibliothèque — {}  ({}/{})",
                    browser.dir.display(),
                    (browser.file_selected + 1).min(browser.files.len()),
                    browser.files.len()
                );
            }
            Part::Hint => {
                *visibility = Visibility::Inherited;
                let hint = if browser.focused {
                    "↑↓ fichiers   Maj+↑↓ dossiers   Entrée entrer   ← parent   F/L → deck   Échap decks"
                } else {
                    "B : clavier vers la bibliothèque   F/L → deck"
                };
                if text.0 != hint {
                    text.0 = hint.into();
                }
            }
            Part::DirRow(index) => {
                let i = browser.dir_scroll + index;
                let entry = browser.dirs.get(i);
                let visible_row = index < visible && entry.is_some();
                *visibility = if visible_row {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if let Some(entry) = entry
                    && visible_row
                {
                    let icon = if entry.path == browser.dir {
                        "▾"
                    } else {
                        "▸"
                    };
                    text.0 = format!("{icon} {}", entry.name);
                    text_color.0 = if i == browser.dir_selected {
                        color::TEXT_PRIMARY
                    } else {
                        color::TEXT_MUTED
                    };
                }
            }
            Part::FileCell { row, col } => {
                let i = browser.file_scroll + row;
                let entry = browser.files.get(i);
                let visible_row = row < visible && entry.is_some();
                *visibility = if visible_row {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if let Some(entry) = entry
                    && visible_row
                {
                    let meta = browser.meta.get(&entry.path);
                    text.0 = match col {
                        0 => {
                            let title = meta
                                .and_then(|m| m.title.as_deref())
                                .unwrap_or(entry.name.as_str());
                            format!("♪ {title}")
                        }
                        1 => meta
                            .and_then(|m| m.artist.clone())
                            .unwrap_or_else(|| "—".into()),
                        2 => meta
                            .and_then(|m| m.bpm)
                            .map_or_else(|| "—".into(), |bpm| format!("{bpm:.0}")),
                        _ => meta
                            .and_then(|m| m.duration_seconds)
                            .map_or_else(|| "…".into(), format_duration),
                    };
                    text_color.0 = if i == browser.file_selected {
                        color::TEXT_PRIMARY
                    } else {
                        color::TEXT_MUTED
                    };
                }
            }
            _ => *visibility = Visibility::Inherited,
        }
    }
}

/// Géométrie du panneau (bande permanente `theme::layout::bands()`) et
/// placement des entités — même calque que le reste de l'écran.
fn place(
    windows: Query<&Window>,
    browser: Res<Browser>,
    mut view: ResMut<BrowserView>,
    mut parts: Query<(&mut Transform, &Part)>,
) {
    let Ok(window) = windows.single() else { return };
    let (w, h) = (window.width(), window.height());
    let bands = layout::bands(w, h);
    let width = w - 2.0 * layout::MARGIN;
    let height = bands.browser_height;
    let center = Vec2::new(0.0, bands.browser_center);
    let left_width = (width * 0.26).clamp(200.0, 380.0);

    let list_top = height * 0.5 - 56.0;
    let footer = -height * 0.5 + 26.0;
    view.rect_center = center;
    view.rect_size = Vec2::new(width, height);
    view.left_width = left_width;
    view.rows_visible = (((list_top - footer - 6.0) / ROW_HEIGHT) as usize)
        .min(MAX_DIR_ROWS)
        .min(MAX_FILE_ROWS);

    let left_x = center.x - width * 0.5;
    let file_x = left_x + left_width + layout::GAP * 2.0;
    let file_width = width - left_width - layout::GAP * 4.0;
    let header_y = center.y + height * 0.5 - 40.0;
    let selected_dir_offset = browser.dir_selected.saturating_sub(browser.dir_scroll) as f32;
    let selected_file_offset = browser.file_selected.saturating_sub(browser.file_scroll) as f32;

    for (mut transform, part) in &mut parts {
        match *part {
            Part::Backdrop => {
                transform.translation = crate::theme::snap(center.extend(10.0));
                transform.scale = Vec3::new(width, height, 1.0);
            }
            Part::Divider => {
                transform.translation =
                    Vec3::new(left_x + left_width + layout::GAP, center.y, 10.4);
                transform.scale = Vec3::new(2.0, height - 16.0, 1.0);
            }
            Part::Title => {
                transform.translation =
                    Vec3::new(left_x + 12.0, center.y + height * 0.5 - 10.0, 11.0);
            }
            Part::Hint => {
                transform.translation =
                    Vec3::new(left_x + 12.0, center.y - height * 0.5 + 6.0, 11.0);
            }
            Part::DirHeader => {
                transform.translation =
                    crate::theme::snap(Vec3::new(left_x + 14.0, header_y, 11.0));
            }
            Part::FileHeader(col) => {
                let x = file_x + COLUMNS[col].1 * file_width;
                transform.translation = crate::theme::snap(Vec3::new(x, header_y, 11.0));
            }
            Part::DirRow(index) => {
                let y = center.y + list_top - (index as f32 + 0.5) * ROW_HEIGHT;
                transform.translation = crate::theme::snap(Vec3::new(left_x + 14.0, y, 11.0));
            }
            Part::FileCell { row, col } => {
                let y = center.y + list_top - (row as f32 + 0.5) * ROW_HEIGHT;
                let x = file_x + COLUMNS[col].1 * file_width;
                transform.translation = crate::theme::snap(Vec3::new(x, y, 11.0));
            }
            Part::DirHighlight => {
                let y = center.y + list_top - (selected_dir_offset + 0.5) * ROW_HEIGHT;
                transform.translation =
                    crate::theme::snap(Vec3::new(left_x + left_width * 0.5, y, 10.5));
                transform.scale = Vec3::new(left_width - 8.0, ROW_HEIGHT - 2.0, 1.0);
            }
            Part::FileHighlight => {
                let y = center.y + list_top - (selected_file_offset + 0.5) * ROW_HEIGHT;
                transform.translation =
                    crate::theme::snap(Vec3::new(file_x + file_width * 0.5 - 8.0, y, 10.5));
                transform.scale = Vec3::new(file_width - 8.0, ROW_HEIGHT - 2.0, 1.0);
            }
            Part::LoadButton(deck) => {
                let x = center.x + width * 0.5 - 150.0 + deck as f32 * 96.0;
                transform.translation = crate::theme::snap(Vec3::new(x, center.y + footer, 11.0));
                transform.scale = Vec3::new(84.0, 26.0, 1.0);
            }
            Part::LoadLabel(deck) => {
                let x = center.x + width * 0.5 - 150.0 + deck as f32 * 96.0;
                transform.translation = crate::theme::snap(Vec3::new(x, center.y + footer, 12.0));
            }
        }
    }
}
