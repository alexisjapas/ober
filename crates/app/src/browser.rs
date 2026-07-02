//! Explorateur de fichiers intégré (couche utilitaire egui, specs §6.1) :
//! navigation dossiers + fichiers audio, chargement direct vers un deck
//! (mêmes workers que la CLI). Bascule avec `B` ; les boutons LOAD, les
//! touches `F`/`L` et l'action MIDI `Load` l'ouvrent aussi. Le dialogue
//! système natif (`rfd`) reste accessible depuis la barre d'outils.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use engine::Deck;

use crate::{LoadSender, picker, spawn_load_worker, theme};

const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "flac", "wav", "ogg", "m4a", "aac"];

pub struct BrowserPlugin;

impl Plugin for BrowserPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Browser::default())
            .add_systems(Update, toggle)
            .add_systems(EguiPrimaryContextPass, draw);
    }
}

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

#[derive(Resource)]
pub struct Browser {
    pub open: bool,
    dir: PathBuf,
    entries: Vec<Entry>,
    dirty: bool,
}

impl Default for Browser {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        // La bibliothèque de l'utilisateur si elle existe, sinon le home.
        let music = home.join("Musique");
        let music_en = home.join("Music");
        let dir = if music.is_dir() {
            music
        } else if music_en.is_dir() {
            music_en
        } else {
            home
        };
        Self {
            open: true,
            dir,
            entries: Vec::new(),
            dirty: true,
        }
    }
}

impl Browser {
    fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.dirty = true;
    }

    fn refresh(&mut self) {
        self.dirty = false;
        self.entries.clear();
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return;
        };
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
                self.entries.push(Entry { name, path, is_dir });
            }
        }
        // Dossiers d'abord, puis fichiers, alphabétique insensible à la casse.
        self.entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut browser: ResMut<Browser>) {
    if keys.just_pressed(KeyCode::KeyB) {
        browser.open = !browser.open;
    }
}

fn draw(
    mut contexts: EguiContexts,
    mut browser: ResMut<Browser>,
    load_tx: Res<LoadSender>,
) -> Result {
    if !browser.open {
        return Ok(());
    }
    if browser.dirty {
        browser.refresh();
    }
    let ctx = contexts.ctx_mut()?;

    let mut open = browser.open;
    let mut navigate_to: Option<PathBuf> = None;
    egui::Window::new("Bibliothèque (B)")
        .open(&mut open)
        .default_size([420.0, 420.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("⬆ parent").clicked()
                    && let Some(parent) = browser.dir.parent()
                {
                    navigate_to = Some(parent.to_path_buf());
                }
                if ui.button("⟳").clicked() {
                    browser.dirty = true;
                }
                if ui.button("dialogue système…").clicked() {
                    // rfd (specs §6.3) reste disponible, vers le deck A.
                    picker::open(Deck::A, load_tx.0.clone());
                }
                ui.label(
                    egui::RichText::new(browser.dir.display().to_string())
                        .color(egui::Color32::GRAY)
                        .small(),
                );
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in &browser.entries {
                    ui.horizontal(|ui| {
                        if entry.is_dir {
                            if ui.button(format!("📁 {}", entry.name)).clicked() {
                                navigate_to = Some(entry.path.clone());
                            }
                        } else {
                            if ui.small_button("→ A").clicked() {
                                spawn_load_worker(entry.path.clone(), Deck::A, load_tx.0.clone());
                            }
                            if ui.small_button("→ B").clicked() {
                                spawn_load_worker(entry.path.clone(), Deck::B, load_tx.0.clone());
                            }
                            ui.label(&entry.name);
                        }
                    });
                }
                if browser.entries.is_empty() {
                    ui.label(
                        egui::RichText::new("aucun dossier ni fichier audio ici")
                            .color(egui::Color32::GRAY),
                    );
                }
            });
        });
    browser.open = open;
    if let Some(dir) = navigate_to {
        browser.navigate(dir);
    }
    // Cohérence visuelle minimale avec le thème (le panneau F12 fait le
    // gros du style ; ici on garde les visuals globaux).
    let _ = theme::color::SURFACE;
    Ok(())
}
