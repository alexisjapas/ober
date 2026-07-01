//! Tests d'intégration du décodage sur des WAV générés (pas de fixtures
//! binaires dans le dépôt). Les autres formats (MP3, FLAC…) sont couverts
//! indirectement par symphonia ; un corpus réel arrive au M5 pour le BPM.

use std::path::PathBuf;

use decode::{CHANNELS, TARGET_SAMPLE_RATE, decode_file};

fn tmp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ober-decode-{}-{name}", std::process::id()))
}

fn write_wav(path: &PathBuf, rate: u32, channels: u16, seconds: f32, freq: f32) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    let frames = (seconds * rate as f32) as usize;
    for i in 0..frames {
        let s = (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin() * 0.8;
        let v = (s * f32::from(i16::MAX)) as i16;
        for _ in 0..channels {
            writer.write_sample(v).unwrap();
        }
    }
    writer.finalize().unwrap();
}

#[test]
fn wav_44k_stereo_resample_en_48k() {
    let path = tmp_path("44k-stereo.wav");
    write_wav(&path, 44_100, 2, 0.5, 440.0);

    let track = decode_file(&path).expect("décodage");
    let _ = std::fs::remove_file(&path);

    assert!(!track.truncated);
    let expected = (0.5 * f64::from(TARGET_SAMPLE_RATE)) as i64;
    let frames = track.frames() as i64;
    assert!(
        (frames - expected).abs() <= 64,
        "frames = {frames}, attendu ≈ {expected}"
    );
    assert!(track.samples.iter().all(|s| s.is_finite()));
    let peak = track.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.5, "peak = {peak}");
}

#[test]
fn wav_mono_duplique_en_stereo() {
    let path = tmp_path("48k-mono.wav");
    write_wav(&path, 48_000, 1, 0.2, 220.0);

    let track = decode_file(&path).expect("décodage");
    let _ = std::fs::remove_file(&path);

    assert_eq!(track.samples.len() % CHANNELS, 0);
    for frame in track.samples.chunks_exact(CHANNELS) {
        assert_eq!(frame[0], frame[1]);
    }
}

#[test]
fn fichier_non_audio_est_une_erreur() {
    let path = tmp_path("garbage.bin");
    std::fs::write(&path, [0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE, 0x42]).unwrap();

    let result = decode_file(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err());
}
