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

pub mod beatgrid;

pub use beatgrid::analyze_track;

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

/// Bus d'analyseurs temps réel (specs §4.2) : alimenté par le tap audio
/// post-mix, il distribue chaque bloc à tous les analyseurs enregistrés.
/// v0.1 n'en branche qu'un (niveaux) ; c'est la fondation des
/// visualisations futures (spectrogramme, chroma, phase entre decks…).
#[derive(Default)]
pub struct AnalyzerBus {
    analyzers: Vec<Box<dyn Analyzer>>,
}

impl AnalyzerBus {
    pub fn register(&mut self, analyzer: Box<dyn Analyzer>) {
        self.analyzers.push(analyzer);
    }

    /// Passe un bloc (stéréo entrelacé) à tous les analyseurs et pousse les
    /// trames produites dans `sink`.
    pub fn process(&mut self, block: &[f32], sink: &mut Vec<AnalysisFrame>) {
        for analyzer in &mut self.analyzers {
            if let Some(frame) = analyzer.process(block) {
                sink.push(frame);
            }
        }
    }
}

/// Analyseur de niveaux RMS/peak pour les VU-mètres (v0.1) : accumule sur
/// une fenêtre fixe puis émet une trame.
pub struct LevelsAnalyzer {
    window_frames: usize,
    count: usize,
    sum_sq: [f64; 2],
    peak: [f32; 2],
}

impl LevelsAnalyzer {
    /// `window_frames` : taille de la fenêtre d'intégration (ex. 2048
    /// frames ≈ 43 ms à 48 kHz — réactif sans scintiller).
    pub fn new(window_frames: usize) -> Self {
        Self {
            window_frames: window_frames.max(1),
            count: 0,
            sum_sq: [0.0; 2],
            peak: [0.0; 2],
        }
    }
}

impl Analyzer for LevelsAnalyzer {
    fn process(&mut self, block: &[f32]) -> Option<AnalysisFrame> {
        for frame in block.chunks_exact(2) {
            self.sum_sq[0] += f64::from(frame[0] * frame[0]);
            self.sum_sq[1] += f64::from(frame[1] * frame[1]);
            self.peak[0] = self.peak[0].max(frame[0].abs());
            self.peak[1] = self.peak[1].max(frame[1].abs());
            self.count += 1;
        }
        if self.count < self.window_frames {
            return None;
        }
        let n = self.count as f64;
        let frame = AnalysisFrame::Levels {
            rms: [
                (self.sum_sq[0] / n).sqrt() as f32,
                (self.sum_sq[1] / n).sqrt() as f32,
            ],
            peak: self.peak,
        };
        self.count = 0;
        self.sum_sq = [0.0; 2];
        self.peak = [0.0; 2];
        Some(frame)
    }
}

/// Filtre passe-bas un pôle (offline, pour le découpage en bandes du
/// waveform summary — les biquads précis restent dans `engine::dsp`).
struct OnePole {
    alpha: f32,
    state: f32,
}

impl OnePole {
    fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let alpha = 1.0 - (-std::f32::consts::TAU * cutoff_hz / sample_rate).exp();
        Self { alpha, state: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        self.state += self.alpha * (x - self.state);
        self.state
    }
}

/// Waveform summary 3 bandes (specs §4.2) : basses < ~250 Hz, médiums,
/// aigus > ~2,5 kHz (crossovers un pôle — suffisant pour du visuel),
/// min/max/RMS par bande à `points_per_second`. Consommé par le rendu M6.
pub fn compute_summary(
    samples_interleaved: &[f32],
    sample_rate: u32,
    points_per_second: u32,
) -> WaveformSummary {
    let mut summary = WaveformSummary {
        points_per_second,
        ..WaveformSummary::default()
    };
    let frames = samples_interleaved.len() / 2;
    if frames == 0 || sample_rate == 0 || points_per_second == 0 {
        return summary;
    }
    let fs = sample_rate as f32;
    let mut lp_low = OnePole::new(250.0, fs);
    let mut lp_high = OnePole::new(2_500.0, fs);
    let window = (sample_rate / points_per_second).max(1) as usize;

    for chunk in samples_interleaved.chunks(window * 2) {
        let mut acc = [(f32::MAX, f32::MIN, 0.0f64); 3];
        let n = (chunk.len() / 2).max(1) as f64;
        for frame in chunk.chunks_exact(2) {
            let mono = (frame[0] + frame[1]) * 0.5;
            let low = lp_low.process(mono);
            let below_high = lp_high.process(mono);
            let bands = [low, below_high - low, mono - below_high];
            for (a, b) in acc.iter_mut().zip(bands) {
                a.0 = a.0.min(b);
                a.1 = a.1.max(b);
                a.2 += f64::from(b * b);
            }
        }
        for (dest, a) in [&mut summary.low, &mut summary.mid, &mut summary.high]
            .into_iter()
            .zip(acc)
        {
            dest.push(WaveformPoint {
                min: a.0,
                max: a.1,
                rms: (a.2 / n).sqrt() as f32,
            });
        }
    }
    summary
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

    fn stereo_sine(freq: f32, seconds: f32, amplitude: f32) -> Vec<f32> {
        let n = (seconds * 48_000.0) as usize;
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = amplitude * (std::f32::consts::TAU * freq * i as f32 / 48_000.0).sin();
            out.push(s);
            out.push(s);
        }
        out
    }

    #[test]
    fn summary_3_bandes_separe_les_registres() {
        let energy = |points: &[WaveformPoint]| -> f64 {
            points.iter().map(|p| f64::from(p.rms * p.rms)).sum::<f64>() / points.len() as f64
        };

        let bass = compute_summary(&stereo_sine(60.0, 1.0, 0.8), 48_000, 1_000);
        assert!(energy(&bass.low) > 20.0 * energy(&bass.high), "basses");

        let treble = compute_summary(&stereo_sine(8_000.0, 1.0, 0.8), 48_000, 1_000);
        assert!(energy(&treble.high) > 20.0 * energy(&treble.low), "aigus");

        let mid = compute_summary(&stereo_sine(1_000.0, 1.0, 0.8), 48_000, 1_000);
        assert!(energy(&mid.mid) > energy(&mid.low), "médiums vs basses");
        assert!(energy(&mid.mid) > energy(&mid.high), "médiums vs aigus");
    }

    #[test]
    fn levels_analyzer_emet_des_trames_rms_peak() {
        let mut bus = AnalyzerBus::default();
        bus.register(Box::new(LevelsAnalyzer::new(2_048)));

        let signal = stereo_sine(440.0, 0.1, 0.5); // 4 800 frames
        let mut frames = Vec::new();
        for block in signal.chunks(512) {
            bus.process(block, &mut frames);
        }
        assert_eq!(frames.len(), 2, "4 800 frames / fenêtre 2 048 → 2 trames");
        let AnalysisFrame::Levels { rms, peak } = frames[0];
        assert!(
            (rms[0] - 0.5 / std::f32::consts::SQRT_2).abs() < 0.02,
            "rms = {}",
            rms[0]
        );
        assert!((peak[0] - 0.5).abs() < 0.01, "peak = {}", peak[0]);
    }
}
