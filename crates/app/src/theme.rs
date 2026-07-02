//! Design system d'ober (specs §6.2) : tokens sémantiques consommés par les
//! materials, les textes et (M6b) le style egui. Aucune couleur, durée ou
//! espacement codé en dur ailleurs dans la couche UI.
//!
//! Fonts cibles : Inter (texte) + Phosphor Icons — vendorisées au M6b dans
//! `assets/fonts/` ; en attendant, la police par défaut de Bevy est utilisée
//! via les tailles définies ici.

// Le design system définit l'ensemble du vocabulaire visuel ; certains
// tokens attendent leurs consommateurs (widgets M6b, style egui).
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

/// Échelle typographique (px) — Inter au M6b.
pub mod font {
    pub const TITLE: f32 = 18.0;
    pub const BODY: f32 = 14.0;
    pub const CAPTION: f32 = 11.5;
}

/// Espacements et proportions de l'écran unique (specs §6.3).
pub mod layout {
    /// Marge extérieure, px.
    pub const MARGIN: f32 = 16.0;
    /// Fraction de la hauteur de fenêtre occupée par chaque waveform.
    pub const WAVE_HEIGHT_FRAC: f32 = 0.26;
    /// Fraction de la largeur occupée par les waveforms (les colonnes de
    /// widgets par deck occupent les bords).
    pub const WAVE_WIDTH_FRAC: f32 = 0.74;
    /// Largeur des colonnes de widgets par deck, px.
    pub const SIDE_COLUMN_PX: f32 = 120.0;
    /// Dimensions d'une barre de VU master, px.
    pub const VU_WIDTH: f32 = 14.0;
    pub const VU_HEIGHT: f32 = 120.0;
    pub const VU_GAP: f32 = 6.0;
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

/// Convertit une couleur du thème en `Vec4` linéaire pour un uniform WGSL.
pub fn to_linear_vec4(color: Color) -> Vec4 {
    let linear = color.to_linear();
    Vec4::new(linear.red, linear.green, linear.blue, linear.alpha)
}
