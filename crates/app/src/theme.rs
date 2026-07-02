//! Design system d'ober (specs §6.2) : tokens sémantiques consommés par les
//! materials, les textes et le style egui (panel.rs). Aucune couleur, durée ou
//! espacement codé en dur ailleurs dans la couche UI.
//!
//! Fonts : Inter (texte) + Phosphor Icons — vendorisées et embarquées par
//! `fonts.rs` (crates/app/src/fonts/), aux tailles définies ici.

// Le design system définit l'ensemble du vocabulaire visuel ; certains
// tokens attendent encore leurs consommateurs (itérations design à venir).
#![allow(dead_code)]

use bevy::prelude::*;

/// Palette sémantique (sRGB).
pub mod color {
    use bevy::color::{Color, Srgba};

    const fn srgb(red: f32, green: f32, blue: f32) -> Color {
        Color::Srgba(Srgba {
            red,
            green,
            blue,
            alpha: 1.0,
        })
    }

    /// Fond de la fenêtre.
    pub const BACKGROUND: Color = srgb(0.024, 0.027, 0.039);
    /// Fond des zones de contenu (waveforms, VU).
    pub const SURFACE: Color = srgb(0.045, 0.05, 0.07);
    /// Panneaux légèrement surélevés (zones de widgets).
    pub const SURFACE_RAISED: Color = srgb(0.062, 0.068, 0.094);

    pub const TEXT_PRIMARY: Color = srgb(0.92, 0.93, 0.96);
    pub const TEXT_MUTED: Color = srgb(0.55, 0.58, 0.66);
    pub const ALERT: Color = srgb(1.0, 0.36, 0.32);

    /// Accents par deck (A : bleu, B : orange).
    pub const DECK_A: Color = srgb(0.35, 0.72, 1.0);
    pub const DECK_B: Color = srgb(1.0, 0.62, 0.25);

    pub const PLAYHEAD: Color = srgb(0.95, 0.96, 1.0);
    pub const BEATGRID: Color = srgb(0.75, 0.78, 0.88);

    /// Teintes des bandes de fréquences des waveforms (basses/médiums/aigus).
    pub const WAVE_LOW: Color = srgb(0.85, 0.35, 0.35);
    pub const WAVE_MID: Color = srgb(0.35, 0.75, 0.45);
    pub const WAVE_HIGH: Color = srgb(0.55, 0.75, 1.0);

    /// Zones des VU-mètres.
    pub const VU_OK: Color = srgb(0.30, 0.78, 0.42);
    pub const VU_WARN: Color = srgb(0.95, 0.78, 0.25);
    pub const VU_CLIP: Color = srgb(1.0, 0.32, 0.28);

    /// Widgets (boutons, pistes de sliders, curseurs).
    pub const WIDGET_BG: Color = srgb(0.10, 0.11, 0.15);
    pub const THUMB: Color = srgb(0.80, 0.82, 0.88);
}

/// Échelle typographique (px), rendue en Inter.
pub mod font {
    pub const TITLE: f32 = 18.0;
    pub const BODY: f32 = 14.0;
    pub const CAPTION: f32 = 11.5;
}

/// Single-screen proportions (specs §6.3). EVERYTHING is expressed in
/// window fractions: the layout rearranges on resize. The two waveforms
/// sit one above the other (A/B visual beat comparison), controls below,
/// and the library is a permanent band on the bottom half — same layer as
/// everything else, never an overlay.
///
/// ```text
/// header (deck texts)            HEADER_FRAC
/// waveform A                     WAVE_HEIGHT_FRAC
/// waveform B                     WAVE_HEIGHT_FRAC
/// controls band                  CONTROLS_FRAC
/// library (two panes)            (the rest)
/// status bar                     STATUS_FRAC
/// ```
pub mod layout {
    /// Marge extérieure, px.
    pub const MARGIN: f32 = 16.0;
    /// Espacement standard entre éléments, px.
    pub const GAP: f32 = 8.0;
    pub const HEADER_FRAC: f32 = 0.07;
    pub const STATUS_FRAC: f32 = 0.05;
    /// Fraction de la hauteur occupée par chaque waveform.
    pub const WAVE_HEIGHT_FRAC: f32 = 0.15;
    /// Fraction de la hauteur occupée par la bande de contrôles.
    pub const CONTROLS_FRAC: f32 = 0.15;
    /// Fraction de la largeur occupée par les waveforms.
    pub const WAVE_WIDTH_FRAC: f32 = 0.96;
    /// Largeur d'une barre de VU master, px.
    pub const VU_WIDTH: f32 = 14.0;
    pub const VU_GAP: f32 = 6.0;

    /// Vertical bands computed for a window size (Bevy 2D centered frame:
    /// +y is up).
    #[derive(Debug, Clone, Copy)]
    pub struct Bands {
        /// Vertical centers of the stacked waveforms (A above B).
        pub wave_center: [f32; 2],
        pub wave_height: f32,
        pub wave_width: f32,
        /// Controls band below the waveforms.
        pub controls_center: f32,
        pub controls_height: f32,
        /// Permanent library band: below the controls, above the status
        /// bar — roughly the bottom half of the window.
        pub browser_center: f32,
        pub browser_height: f32,
    }

    pub fn bands(width: f32, height: f32) -> Bands {
        let wave_height = height * WAVE_HEIGHT_FRAC;
        let wave_a = height * (0.5 - HEADER_FRAC) - wave_height * 0.5;
        let wave_b = wave_a - wave_height - GAP;
        let controls_top = wave_b - wave_height * 0.5 - GAP;
        let controls_height = (height * CONTROLS_FRAC).max(60.0);
        let controls_bottom = controls_top - controls_height;
        let browser_top = controls_bottom - GAP;
        let browser_bottom = -(height * (0.5 - STATUS_FRAC));
        Bands {
            wave_center: [wave_a, wave_b],
            wave_height,
            wave_width: width * WAVE_WIDTH_FRAC,
            controls_center: (controls_top + controls_bottom) * 0.5,
            controls_height,
            browser_center: (browser_top + browser_bottom) * 0.5,
            browser_height: (browser_top - browser_bottom).max(100.0),
        }
    }
}

/// Courbes d'easing centralisées (specs §6.2) : toute animation passe par
/// ici, jamais de constante en dur dans les systèmes.
pub mod easing {
    /// Décroissance du peak-hold des VU, en unités de niveau par seconde.
    pub const VU_PEAK_DECAY_PER_S: f32 = 0.6;
    /// Constante de temps du lissage montant des VU (attaque rapide).
    pub const VU_ATTACK_TAU_S: f32 = 0.010;
    /// Constante de temps du retour des VU (relâchement doux).
    pub const VU_RELEASE_TAU_S: f32 = 0.120;

    pub fn ease_out_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }

    /// Coefficient de lissage exponentiel pour un pas `dt` et une constante
    /// de temps `tau`.
    pub fn smoothing_alpha(dt: f32, tau: f32) -> f32 {
        if tau <= 0.0 {
            1.0
        } else {
            1.0 - (-dt / tau).exp()
        }
    }
}

/// Snaps a translation to whole logical pixels. The percentage layout
/// yields fractional positions; texts and hard quad edges sitting between
/// pixels render soft — one rounding at placement time keeps them crisp
/// (`z` untouched, it orders the layers).
pub fn snap(translation: Vec3) -> Vec3 {
    Vec3::new(translation.x.round(), translation.y.round(), translation.z)
}

/// Convertit une couleur du thème en `Vec4` linéaire pour un uniform WGSL.
pub fn to_linear_vec4(color: Color) -> Vec4 {
    let linear = color.to_linear();
    Vec4::new(linear.red, linear.green, linear.blue, linear.alpha)
}
