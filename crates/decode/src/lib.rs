//! Offline audio decoding: symphonia (MP3, FLAC, WAV, OGG Vorbis, AAC/M4A)
//! then rubato resampling to the engine's interleaved stereo f32 format at
//! the rate of the opened output stream (specs §4.1 — 48 kHz preferred,
//! 44.1 kHz on natively 44.1-only devices, cf. docs/latency.md). Runs in a
//! worker thread, never in the audio callback. No Bevy dependency.

mod resample;

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Preferred decode target (specs §3.1). The actual target follows the
/// opened output stream — callers pass `StreamInfo::sample_rate` to
/// [`decode_file`]; this constant is the default for offline uses (tests).
pub const TARGET_SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// Piste entièrement décodée en mémoire (POC : pistes ≤ 15 min, specs §3.4 —
/// le streaming par chunks est une évolution v0.2).
pub struct DecodedTrack {
    /// Interleaved stereo f32 at `sample_rate`. Mono duplicated on both
    /// channels.
    pub samples: Vec<f32>,
    /// Rate the track was resampled to (the engine's stream rate).
    pub sample_rate: u32,
    /// Vrai si le fichier était tronqué ou corrompu en cours de flux : on
    /// garde ce qui a été décodé et on le signale à l'UI (specs §4.1).
    pub truncated: bool,
}

impl DecodedTrack {
    pub fn frames(&self) -> usize {
        self.samples.len() / CHANNELS
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.sample_rate)
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

/// Lightweight header probe for the library browser (specs §6.3): duration,
/// technical format and tags, **without decoding any audio**. Best-effort —
/// a field the container doesn't expose is simply `None`.
#[derive(Debug, Default, Clone)]
pub struct ProbeInfo {
    pub duration_seconds: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<usize>,
    pub artist: Option<String>,
    pub title: Option<String>,
}

/// Probes `path` headers. `None` when the file isn't recognized audio.
pub fn probe_info(path: &Path) -> Option<ProbeInfo> {
    use symphonia::core::meta::StandardTag;

    let file = File::open(path).ok()?;
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
        .ok()?;

    let track = format.default_track(TrackType::Audio)?;
    let params = track.codec_params.as_ref().and_then(|p| p.audio());
    let sample_rate = params.and_then(|p| p.sample_rate);
    let channels = params.and_then(|p| p.channels.as_ref().map(|c| c.count()));
    let duration_seconds = match (track.num_frames, sample_rate) {
        (Some(frames), Some(rate)) if rate > 0 => Some(frames as f64 / f64::from(rate)),
        _ => None,
    };

    let mut artist = None;
    let mut title = None;
    if let Some(revision) = format.metadata().current() {
        for tag in &revision.media.tags {
            match &tag.std {
                Some(StandardTag::Artist(v)) => artist = Some(v.to_string()),
                Some(StandardTag::TrackTitle(v)) => title = Some(v.to_string()),
                _ => {}
            }
        }
    }

    Some(ProbeInfo {
        duration_seconds,
        sample_rate,
        channels,
        artist,
        title,
    })
}

/// Decodes the whole file in memory, then resamples to `target_rate` (the
/// engine's stream rate) when needed. A truncated file yields a partial
/// track flagged by `truncated`, never an error (as long as at least one
/// packet was decoded).
pub fn decode_file(path: &Path, target_rate: u32) -> Result<DecodedTrack, DecodeError> {
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
    let samples = if src_rate == target_rate {
        stereo
    } else {
        resample::resample_stereo(&stereo, src_rate, target_rate)?
    };

    Ok(DecodedTrack {
        samples,
        sample_rate: target_rate,
        truncated,
    })
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
