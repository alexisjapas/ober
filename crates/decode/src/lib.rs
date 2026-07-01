//! Décodage audio offline : symphonia (MP3, FLAC, WAV, OGG Vorbis, AAC/M4A)
//! puis resampling rubato vers le format interne f32 48 kHz stéréo entrelacé
//! (specs §4.1). Tourne dans un thread worker, jamais dans le callback audio.
//! Aucune dépendance Bevy.

mod resample;

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Tout fichier est resamplé vers ce taux au décodage (specs §3.1).
pub const TARGET_SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// Piste entièrement décodée en mémoire (POC : pistes ≤ 15 min, specs §3.4 —
/// le streaming par chunks est une évolution v0.2).
pub struct DecodedTrack {
    /// f32 stéréo entrelacé à 48 kHz. Mono dupliqué sur les deux canaux.
    pub samples: Vec<f32>,
    /// Vrai si le fichier était tronqué ou corrompu en cours de flux : on
    /// garde ce qui a été décodé et on le signale à l'UI (specs §4.1).
    pub truncated: bool,
}

impl DecodedTrack {
    pub fn frames(&self) -> usize {
        self.samples.len() / CHANNELS
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(TARGET_SAMPLE_RATE)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("format non supporté ou non reconnu : {0}")]
    UnsupportedFormat(String),
    #[error("aucune piste audio décodable")]
    NoAudioTrack,
    #[error("aucune donnée audio décodable")]
    Empty,
    #[error("resampling : {0}")]
    Resample(String),
}

/// Décode l'intégralité du fichier en mémoire, puis resample vers 48 kHz si
/// nécessaire. Un fichier tronqué produit une piste partielle signalée par
/// `truncated`, jamais une erreur (tant qu'au moins un paquet a été décodé).
pub fn decode_file(path: &Path) -> Result<DecodedTrack, DecodeError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| DecodeError::UnsupportedFormat(e.to_string()))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or(DecodeError::NoAudioTrack)?
        .clone();

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
        .map_err(|e| DecodeError::UnsupportedFormat(e.to_string()))?;

    // Décodage intégral vers f32 entrelacé, au format du fichier source.
    let mut src: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    let mut src_rate = 0u32;
    let mut src_channels = 0usize;
    let mut truncated = false;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break, // fin de flux propre
            // Fichier tronqué : on garde ce qui a été décodé (specs §4.1).
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                truncated = true;
                break;
            }
            Err(_) => {
                truncated = true;
                break;
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = buf.spec();
                src_rate = spec.rate();
                src_channels = spec.channels().count();
                let n = buf.samples_interleaved();
                scratch.resize(n, 0.0);
                buf.copy_to_slice_interleaved::<f32, _>(&mut scratch[..n]);
                src.extend_from_slice(&scratch[..n]);
            }
            // Paquet corrompu isolé : on le saute et on continue.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => {
                truncated = true;
                break;
            }
        }
    }

    if src.is_empty() || src_channels == 0 || src_rate == 0 {
        return Err(DecodeError::Empty);
    }

    let stereo = to_stereo(src, src_channels);
    let samples = if src_rate == TARGET_SAMPLE_RATE {
        stereo
    } else {
        resample::resample_stereo_48k(&stereo, src_rate)?
    };

    Ok(DecodedTrack { samples, truncated })
}

/// Mono → duplication sur les deux canaux ; multicanal → deux premiers
/// canaux (suffisant pour le POC).
fn to_stereo(interleaved: Vec<f32>, channels: usize) -> Vec<f32> {
    match channels {
        2 => interleaved,
        1 => {
            let mut out = Vec::with_capacity(interleaved.len() * 2);
            for s in interleaved {
                out.push(s);
                out.push(s);
            }
            out
        }
        n => interleaved
            .chunks_exact(n)
            .flat_map(|frame| [frame[0], frame[1]])
            .collect(),
    }
}
