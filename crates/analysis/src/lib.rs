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

/// Résumé mono-bande min/max/RMS d'une piste stéréo entrelacée (mix L+R),
/// à `points_per_second` points. Fondation du rendu waveform (uploadé une
/// fois en texture GPU, specs §6.1) ; la version 3 bandes filtrées arrive
/// au M5.
pub fn compute_overview(
    samples_interleaved: &[f32],
    sample_rate: u32,
    points_per_second: u32,
) -> Vec<WaveformPoint> {
    let frames = samples_interleaved.len() / 2;
    if frames == 0 || sample_rate == 0 || points_per_second == 0 {
        return Vec::new();
    }
    let window = (sample_rate / points_per_second).max(1) as usize;
    let mut points = Vec::with_capacity(frames / window + 1);
    for chunk in samples_interleaved.chunks(window * 2) {
        let mut point = WaveformPoint {
            min: f32::MAX,
            max: f32::MIN,
            rms: 0.0,
        };
        let mut sum_sq = 0.0f64;
        let n = (chunk.len() / 2).max(1);
        for frame in chunk.chunks_exact(2) {
            let mono = (frame[0] + frame[1]) * 0.5;
            point.min = point.min.min(mono);
            point.max = point.max.max(mono);
            sum_sq += f64::from(mono * mono);
        }
        point.rms = (sum_sq / n as f64).sqrt() as f32;
        points.push(point);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_d_un_sinus() {
        // 1 s de sinus 440 Hz d'amplitude 0,8 : chaque fenêtre de 48 samples
        // (1000 pts/s) couvre ~0,44 période — les extrema globaux et le RMS
        // global restent caractéristiques.
        let mut samples = Vec::new();
        for i in 0..48_000 {
            let s = 0.8 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin();
            samples.push(s);
            samples.push(s);
        }
        let points = compute_overview(&samples, 48_000, 1_000);
        assert_eq!(points.len(), 1_000);

        let min = points.iter().fold(0.0f32, |m, p| m.min(p.min));
        let max = points.iter().fold(0.0f32, |m, p| m.max(p.max));
        assert!((min + 0.8).abs() < 0.01, "min = {min}");
        assert!((max - 0.8).abs() < 0.01, "max = {max}");

        let mean_rms = points.iter().map(|p| f64::from(p.rms)).sum::<f64>() / 1_000.0;
        assert!(
            (mean_rms - 0.8 / std::f64::consts::SQRT_2).abs() < 0.05,
            "rms moyen = {mean_rms}"
        );
    }

    #[test]
    fn overview_vide() {
        assert!(compute_overview(&[], 48_000, 1_000).is_empty());
    }
}
