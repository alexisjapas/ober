//! Moteur audio temps réel de dj-mix.
//!
//! Cette crate ne dépend JAMAIS de Bevy (frontière architecturale, specs
//! §1.4/§2.4, vérifiée en CI par `scripts/check-bevy-boundary.sh`). Elle doit
//! compiler et être testable seule : `cargo test -p engine`.
//!
//! # Règles absolues du callback audio (specs §2.2)
//!
//! Le callback `cpal` ne doit jamais :
//! - allouer (`Box`, `Vec::push`, `String`, `format!`…) ;
//! - prendre un `Mutex`/`RwLock` (y compris implicitement via `log!`) ;
//! - faire d'I/O (fichier, réseau, stdout) ;
//! - bloquer sur un channel (`try_recv`/`pop` non bloquants uniquement) ;
//! - désallouer un buffer de piste — renvoi au worker via le canal de
//!   récupération mémoire (specs §2.3).
//!
//! Toute la mémoire est pré-allouée hors callback puis transférée par échange
//! de pointeur (`Arc<TrackBuffer>`) via channel lock-free. La feature
//! `rt-checks` installe l'allocateur traqué d'`assert_no_alloc` et fait
//! paniquer tout le processus si le callback alloue (debug uniquement).

pub mod command;
pub mod graph;
pub mod snapshot;
pub mod stream;
pub mod track;

pub use command::EngineCommand;
pub use graph::{AudioGraph, EnginePorts};
pub use snapshot::{DeckSnapshot, EngineSnapshot};
pub use stream::{Engine, EngineError, StreamInfo};
pub use track::TrackBuffer;

/// Format interne : f32, 48 kHz, stéréo entrelacé (specs §3.1).
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Taille de buffer cible en frames (specs §3.1 : 128–256). Clampée à la
/// plage supportée par le périphérique (le fallback 512 en découle).
pub const TARGET_BUFFER_FRAMES: u32 = 256;

/// Détection d'allocation dans le callback (specs §7). Actif uniquement en
/// debug : en release l'allocateur traqué est transparent.
#[cfg(feature = "rt-checks")]
#[global_allocator]
static RT_CHECK_ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// Identifiant de deck. Le POC en gère deux (specs §1.2) ; l'architecture ne
/// doit pas rendre ce nombre difficile à augmenter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Deck {
    A,
    B,
}

impl Deck {
    pub const ALL: [Deck; 2] = [Deck::A, Deck::B];

    pub fn index(self) -> usize {
        match self {
            Deck::A => 0,
            Deck::B => 1,
        }
    }
}
