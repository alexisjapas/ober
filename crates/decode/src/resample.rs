//! Resampling offline haute qualité vers 48 kHz : rubato, interpolation sinc
//! (specs §4.1 — l'équivalent de l'ancien `SincFixedIn` est `Async` +
//! `FixedAsync::Input` depuis rubato 3).

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction, calculate_cutoff,
};

use crate::{CHANNELS, DecodeError, TARGET_SAMPLE_RATE};

const SINC_LEN: usize = 256;
const CHUNK_FRAMES: usize = 4096;

fn err(e: impl std::fmt::Display) -> DecodeError {
    DecodeError::Resample(e.to_string())
}

/// Resample un buffer stéréo entrelacé de `src_rate` vers 48 kHz.
/// Qualité haute (sinc 256 points, interpolation cubique) — c'est offline.
pub(crate) fn resample_stereo_48k(input: &[f32], src_rate: u32) -> Result<Vec<f32>, DecodeError> {
    debug_assert_eq!(input.len() % CHANNELS, 0);
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let ratio = f64::from(TARGET_SAMPLE_RATE) / f64::from(src_rate);
    let params = SincInterpolationParameters {
        sinc_len: SINC_LEN,
        f_cutoff: calculate_cutoff(SINC_LEN, WindowFunction::BlackmanHarris2),
        interpolation: SincInterpolationType::Cubic,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = Async::<f32>::new_sinc(
        ratio,
        1.0,
        &params,
        CHUNK_FRAMES,
        CHANNELS,
        FixedAsync::Input,
    )
    .map_err(err)?;

    let in_frames_total = input.len() / CHANNELS;
    let expected_out = (in_frames_total as f64 * ratio).round() as usize;
    // Le filtre introduit un délai : on produit `delay` frames de plus puis on
    // les retire en tête pour garder l'alignement temporel de la piste.
    let delay = resampler.output_delay();

    let out_max = resampler.output_frames_max();
    let mut out_scratch = vec![0.0f32; out_max * CHANNELS];
    let mut out: Vec<f32> = Vec::with_capacity((expected_out + delay + out_max) * CHANNELS);

    let mut consumed = 0usize;
    // Consomme l'entrée par chunks fixes ; la fin de piste puis la vidange du
    // délai du filtre passent par `partial_len` (complétées en silence).
    while out.len() < (expected_out + delay) * CHANNELS {
        let needed = resampler.input_frames_next();
        let available = (in_frames_total - consumed).min(needed);
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: (available < needed).then_some(available),
            active_channels_mask: None,
        };

        let in_slice = &input[consumed * CHANNELS..(consumed + available) * CHANNELS];
        let in_adapter = InterleavedSlice::new(in_slice, CHANNELS, available).map_err(err)?;
        let mut out_adapter =
            InterleavedSlice::new_mut(&mut out_scratch[..], CHANNELS, out_max).map_err(err)?;

        let (_frames_in, frames_out) = resampler
            .process_into_buffer(&in_adapter, &mut out_adapter, Some(&indexing))
            .map_err(err)?;

        if frames_out == 0 && available == 0 {
            return Err(DecodeError::Resample(
                "le resampler ne produit plus de données".into(),
            ));
        }
        consumed += available;
        out.extend_from_slice(&out_scratch[..frames_out * CHANNELS]);
    }

    let start = delay * CHANNELS;
    let end = (delay + expected_out) * CHANNELS;
    Ok(out[start..end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_44k_vers_48k_conserve_duree_et_frequence() {
        // 0,5 s de sinus 440 Hz à 44,1 kHz, stéréo.
        let src_rate = 44_100u32;
        let frames = (src_rate / 2) as usize;
        let mut input = Vec::with_capacity(frames * CHANNELS);
        for i in 0..frames {
            let s = (std::f32::consts::TAU * 440.0 * i as f32 / src_rate as f32).sin() * 0.8;
            input.push(s);
            input.push(s);
        }

        let out = resample_stereo_48k(&input, src_rate).unwrap();
        let out_frames = out.len() / CHANNELS;
        let expected = (frames as f64 * 48_000.0 / 44_100.0).round() as usize;
        assert_eq!(out_frames, expected);
        assert!(out.iter().all(|s| s.is_finite()));

        // Fréquence préservée : comptage de passages par zéro sur le canal
        // gauche, en ignorant les bords (transitoires du filtre).
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let core = &left[2_000..left.len() - 2_000];
        let crossings = core
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        let seconds = core.len() as f64 / 48_000.0;
        let freq = crossings as f64 / seconds;
        assert!((freq - 440.0).abs() < 5.0, "fréquence estimée : {freq} Hz");
    }
}
