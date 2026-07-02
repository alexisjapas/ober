//! Moteur audio temps réel d'ober.
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
pub mod dsp;
pub mod graph;
pub mod jog;
pub mod snapshot;
pub mod stream;
pub mod track;

pub use command::{EngineCommand, JogParams};
pub use dsp::EqBand;
pub use graph::{AudioGraph, EnginePorts};
pub use jog::JogRuntime;
pub use snapshot::{DeckSnapshot, EngineSnapshot};
pub use stream::{Engine, EngineConfig, EngineError, StreamInfo};
pub use track::TrackBuffer;

/// Preferred sample rate (specs §3.1: f32, interleaved stereo, 48 kHz). The
/// engine actually runs at the rate of the output stream it managed to open:
/// 44.1 kHz-only devices (e.g. the DJControl Inpulse 200 MK2) are driven at
/// their native rate to avoid the ALSA plug resampler and its ~23 ms buffer
/// (cf. docs/latency.md). Everything downstream — decode target, EQ
/// coefficients, jog model, seek conversions — derives from
/// [`StreamInfo::sample_rate`], never from this constant.
pub const PREFERRED_SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Taille de buffer cible en frames (specs §3.1 : 128–256). Clampée à la
/// plage supportée par le périphérique (le fallback 512 en découle).
pub const TARGET_BUFFER_FRAMES: u32 = 256;
/// Taille maximale d'un bloc de callback (dimensionne les buffers scratch
/// pré-alloués du graphe — au-delà, le bloc est tronqué).
pub const MAX_BLOCK_FRAMES: usize = 8_192;
/// Excursion maximale du varispeed (±16 %, specs §1.2).
pub const MAX_PITCH_RATIO: f64 = 0.16;

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
