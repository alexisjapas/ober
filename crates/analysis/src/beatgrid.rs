//! Détection BPM + beatgrid offline (specs §4.2) :
//!
//! 1. **onsets** : flux d'énergie spectrale rectifié (STFT rustfft,
//!    fenêtres de 1024 samples, hop 512, Hann) → courbe de nouveauté à
//!    ~93,75 Hz ;
//! 2. **tempo** : autocorrélation de la nouveauté sur la plage 60–200 BPM,
//!    renfort harmonique léger (anti erreur d'octave), puis raffinement du
//!    pic par **repliement de phase** (la nouveauté est repliée modulo la
//!    période candidate ; la bonne période concentre tous les beats dans le
//!    même bin — l'erreur s'accumulant sur toute la piste, la précision
//!    dépasse largement 0,01 BPM) ;
//! 3. **phase** : premier beat par maximisation de l'énergie de la
//!    nouveauté alignée sur la grille.
//!
//! Grille fixe (tempo constant) — suffisant pour le POC.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::TrackAnalysis;

const FFT_SIZE: usize = 1024;
const HOP: usize = 512;
pub const MIN_BPM: f64 = 60.0;
pub const MAX_BPM: f64 = 200.0;

/// Analyse une piste stéréo entrelacée. `None` si la piste est trop courte
/// (< ~4 s) ou sans périodicité exploitable.
pub fn analyze_track(samples_interleaved: &[f32], sample_rate: u32) -> Option<TrackAnalysis> {
    let novelty = spectral_flux(samples_interleaved)?;
    let frame_rate = f64::from(sample_rate) / HOP as f64;
    let period = best_period(&novelty, frame_rate)?;
    let bpm = (60.0 * frame_rate / period * 100.0).round() / 100.0;
    let phase_frames = best_phase(&novelty, period);
    Some(TrackAnalysis {
        bpm,
        first_beat_sample: (phase_frames * HOP as f64).round() as u64,
    })
}

/// Courbe de nouveauté : somme des accroissements de magnitude spectrale
/// entre trames successives (rectification demi-onde).
fn spectral_flux(samples_interleaved: &[f32]) -> Option<Vec<f32>> {
    let mono: Vec<f32> = samples_interleaved
        .chunks_exact(2)
        .map(|f| (f[0] + f[1]) * 0.5)
        .collect();
    if mono.len() < FFT_SIZE * 8 {
        return None;
    }

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let hann: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            let x = std::f32::consts::TAU * i as f32 / FFT_SIZE as f32;
            0.5 * (1.0 - x.cos())
        })
        .collect();

    let n_frames = (mono.len() - FFT_SIZE) / HOP + 1;
    let mut buffer = vec![Complex::new(0.0f32, 0.0); FFT_SIZE];
    let mut prev_magnitude = vec![0.0f32; FFT_SIZE / 2];
    let mut novelty = Vec::with_capacity(n_frames);

    for frame in 0..n_frames {
        let start = frame * HOP;
        for (i, slot) in buffer.iter_mut().enumerate() {
            *slot = Complex::new(mono[start + i] * hann[i], 0.0);
        }
        fft.process(&mut buffer);

        let mut flux = 0.0f32;
        for (bin, prev) in buffer[..FFT_SIZE / 2].iter().zip(prev_magnitude.iter_mut()) {
            let magnitude = bin.norm();
            flux += (magnitude - *prev).max(0.0);
            *prev = magnitude;
        }
        novelty.push(flux);
    }
    Some(novelty)
}

/// Période de battement (en trames de nouveauté, fractionnaire) par
/// autocorrélation + interpolation parabolique.
fn best_period(novelty: &[f32], frame_rate: f64) -> Option<f64> {
    let n = novelty.len();
    let mean = novelty.iter().map(|v| f64::from(*v)).sum::<f64>() / n as f64;
    let x: Vec<f64> = novelty.iter().map(|v| f64::from(*v) - mean).collect();

    let autocorr = |lag: usize| -> f64 {
        if lag >= n {
            return 0.0;
        }
        x[..n - lag]
            .iter()
            .zip(&x[lag..])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / (n - lag) as f64
    };

    let min_lag = (frame_rate * 60.0 / MAX_BPM).floor().max(2.0) as usize;
    let max_lag = (frame_rate * 60.0 / MIN_BPM).ceil() as usize;
    // Au moins ~3 périodes de la plus lente pour une corrélation fiable.
    if n < max_lag * 3 {
        return None;
    }

    let mut best_lag = 0usize;
    let mut best_score = f64::MIN;
    for lag in min_lag..=max_lag {
        // Renfort harmonique : une vraie période corrèle aussi à son double.
        let score = autocorr(lag) + 0.5 * autocorr(2 * lag);
        if score > best_score {
            best_score = score;
            best_lag = lag;
        }
    }
    if best_score <= 0.0 {
        return None; // pas de périodicité (silence, bruit)
    }

    Some(refine_period(novelty, best_lag as f64))
}

/// Raffinement sub-trame de la période : replie la nouveauté modulo chaque
/// période candidate (±1 trame autour du pic entier, pas de 0,002) dans un
/// histogramme de phase — la période exacte maximise la concentration.
fn refine_period(novelty: &[f32], center: f64) -> f64 {
    const BINS: usize = 64;
    const STEP: f64 = 0.002;

    let mut best_period = center;
    let mut best_score = f64::MIN;
    let mut period = (center - 1.0).max(2.0);
    let end = center + 1.0;
    while period <= end {
        let mut histogram = [0.0f64; BINS];
        for (i, v) in novelty.iter().enumerate() {
            let phase = (i as f64 / period).fract();
            histogram[(phase * BINS as f64) as usize % BINS] += f64::from(*v);
        }
        let score = histogram.iter().copied().fold(f64::MIN, f64::max);
        if score > best_score {
            best_score = score;
            best_period = period;
        }
        period += STEP;
    }
    best_period
}

/// Phase du premier beat (en trames de nouveauté) : décalage qui maximise
/// l'énergie de nouveauté échantillonnée sur la grille.
fn best_phase(novelty: &[f32], period: f64) -> f64 {
    let n = novelty.len();
    let steps = period.round().max(1.0) as usize;
    let mut best_phase = 0usize;
    let mut best_sum = f64::MIN;
    for phase in 0..steps {
        let mut sum = 0.0;
        let mut position = phase as f64;
        while (position as usize) < n {
            sum += f64::from(novelty[position as usize]);
            position += period;
        }
        if sum > best_sum {
            best_sum = sum;
            best_phase = phase;
        }
    }
    best_phase as f64
}
