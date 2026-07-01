//! Primitives DSP du moteur (specs §3.3).
//!
//! Règle de partage temps réel / non temps réel :
//! - le **calcul des coefficients** (RBJ, courbes) se fait hors callback —
//!   les commandes moteur transportent des coefficients prêts à l'emploi ;
//! - le **traitement** (`process…`) est sans allocation, sans branche
//!   coûteuse, appelable depuis le callback.

pub mod biquad;
pub mod eq;
pub mod hermite;
pub mod softclip;

pub use biquad::{BiquadCoeffs, BiquadState};
pub use eq::{EqBand, StereoEq, eq_coeffs};
pub use hermite::hermite4;
pub use softclip::soft_clip;
