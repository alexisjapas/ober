//! Décodage audio offline : symphonia (MP3, FLAC, WAV, OGG Vorbis, AAC/M4A)
//! puis resampling rubato vers le format interne f32 48 kHz stéréo entrelacé
//! (specs §4.1). Tourne dans un thread worker, jamais dans le callback audio.
//! Aucune dépendance Bevy.

use std::path::Path;

/// Tout fichier est resamplé vers ce taux au décodage (specs §3.1).
pub const TARGET_SAMPLE_RATE: u32 = 48_000;

/// Piste entièrement décodée en mémoire (POC : pistes ≤ 15 min, specs §3.4 —
/// le streaming par chunks est une évolution v0.2).
pub struct DecodedTrack {
    /// f32 stéréo entrelacé à 48 kHz. Mono dupliqué sur les deux canaux.
    pub samples: Vec<f32>,
    /// Vrai si le fichier était tronqué : on garde ce qui a été décodé et on
    /// le signale à l'UI (specs §4.1).
    pub truncated: bool,
}

impl DecodedTrack {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames() as f64 / TARGET_SAMPLE_RATE as f64
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    #[error("format non supporté ou non reconnu")]
    UnsupportedFormat,
    #[error("aucune donnée audio décodable")]
    Empty,
}

/// Décode l'intégralité du fichier en mémoire. Implémentation : jalon M1.
pub fn decode_file(_path: &Path) -> Result<DecodedTrack, DecodeError> {
    todo!("M1 : symphonia (probe → packets) puis rubato SincFixedIn vers 48 kHz")
}
