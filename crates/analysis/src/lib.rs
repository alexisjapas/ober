//! Analyse audio (specs §4.2) — aucune dépendance Bevy.
//!
//! Deux volets :
//! - **offline** (jalon M5) : BPM + beatgrid (onsets par flux d'énergie
//!   spectrale, autocorrélation 60–200 BPM, résolution 0,01) et waveform
//!   summary 3 bandes ;
//! - **temps réel** : bus d'analyseurs alimenté par le tap audio (§2.3),
//!   côté worker. v0.1 n'implémente que les niveaux RMS/peak, mais le bus est
//!   la fondation des visualisations futures (spectrogramme, chroma,
//!   corrélation de phase, structure).

/// Un point de waveform summary : min/max/RMS sur une fenêtre (~1 ms).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaveformPoint {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

/// Résumé de waveform 3 bandes (basses/médiums/aigus, ~1000 points/s),
/// uploadé côté `app` en textures GPU avec mipmaps 1×/4×/16× (specs §6.1).
#[derive(Debug, Clone, Default)]
pub struct WaveformSummary {
    pub points_per_second: u32,
    pub low: Vec<WaveformPoint>,
    pub mid: Vec<WaveformPoint>,
    pub high: Vec<WaveformPoint>,
}

/// Résultat de l'analyse offline. Grille fixe (tempo constant) — suffisant
/// pour le POC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackAnalysis {
    /// Plage 60–200 BPM, résolution 0,01 BPM.
    pub bpm: f64,
    /// Phase du premier beat, en samples (48 kHz).
    pub first_beat_sample: u64,
}

/// Trame produite par un analyseur temps réel, acheminée vers Bevy par un
/// canal dédié, typée par analyseur.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AnalysisFrame {
    /// v0.1 : niveaux pour les VU-mètres.
    Levels { rms: [f32; 2], peak: [f32; 2] },
    // v0.2+ : Spectrogram, Chroma, PhaseCorrelation, Structure…
}

/// Analyseur temps réel enregistré dynamiquement sur le bus. Les
/// implémentations tournent côté worker, jamais dans le callback audio.
pub trait Analyzer: Send {
    fn process(&mut self, block: &[f32]) -> Option<AnalysisFrame>;
}
