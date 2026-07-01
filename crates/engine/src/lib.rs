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
//! - `drop` un buffer de piste — renvoi au worker via le canal de
//!   récupération mémoire (specs §2.3).
//!
//! Toute la mémoire est pré-allouée hors callback puis transférée par échange
//! de pointeur via channel lock-free. La feature `rt-checks` arme
//! `assert_no_alloc` autour du callback en debug.

pub mod command;
pub mod snapshot;

/// Format interne : f32, 48 kHz, stéréo entrelacé (specs §3.1).
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

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
