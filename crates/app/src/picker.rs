//! File picker natif `rfd` (specs §6.3), backend xdg-portal (aucune
//! dépendance GTK). Le dialogue vit sur son propre thread — jamais de
//! blocage du frame — et le fichier choisi repart par le même chemin que
//! les chargements CLI (worker de décodage → `WorkerMsg`).

use std::sync::mpsc::Sender;

use engine::Deck;

use crate::{WorkerMsg, spawn_load_worker};

/// Ouvre le dialogue pour un deck. Sans sélection : aucun effet.
pub fn open(deck: Deck, tx: Sender<WorkerMsg>) {
    std::thread::Builder::new()
        .name(format!("picker-{deck:?}"))
        .spawn(move || {
            let file = rfd::FileDialog::new()
                .set_title(format!("ober — charger une piste sur le deck {deck:?}"))
                .add_filter("Audio", &["mp3", "flac", "wav", "ogg", "m4a", "aac"])
                .pick_file();
            if let Some(path) = file {
                spawn_load_worker(path, deck, tx);
            }
        })
        .ok();
}
