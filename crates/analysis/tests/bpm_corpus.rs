//! Corpus BPM (specs §7) : clicks générés à tempo connu, tolérance
//! ±0,1 BPM. Des extraits réels pourront compléter le corpus quand des
//! fichiers de test seront versionnés (Git LFS ou fixtures téléchargées).

use analysis::analyze_track;

const FS: u32 = 48_000;

/// Piste de clicks : burst de 3 kHz amorti (~5 ms) sur chaque beat.
fn click_track(bpm: f64, seconds: f64, first_beat_s: f64) -> Vec<f32> {
    let frames = (seconds * f64::from(FS)) as usize;
    let mut mono = vec![0.0f32; frames];
    let beat_period = 60.0 / bpm;
    let mut t = first_beat_s;
    while t < seconds {
        let start = (t * f64::from(FS)) as usize;
        for i in 0..240.min(frames.saturating_sub(start)) {
            let envelope = (-(i as f32) / 40.0).exp();
            mono[start + i] +=
                0.9 * envelope * (std::f32::consts::TAU * 3_000.0 * i as f32 / FS as f32).sin();
        }
        t += beat_period;
    }
    mono.iter().flat_map(|s| [*s, *s]).collect()
}

#[test]
fn corpus_de_clicks_a_tempo_connu() {
    // (bpm, offset du premier beat en secondes)
    for (bpm, offset) in [(120.0, 0.25), (87.5, 0.10), (174.0, 0.00), (60.0, 0.50)] {
        let track = click_track(bpm, 30.0, offset);
        let analysis =
            analyze_track(&track, FS).unwrap_or_else(|| panic!("analyse échouée à {bpm} BPM"));
        assert!(
            (analysis.bpm - bpm).abs() <= 0.1,
            "{bpm} BPM détecté à {} (tolérance ±0,1)",
            analysis.bpm
        );

        // Phase : ce qui compte pour un beatgrid est la phase MODULO la
        // période — le « premier beat » peut être décalé d'une période
        // entière. Tolérance ~43 ms (résolution du hop + étalement du flux).
        let period_samples = 60.0 / bpm * f64::from(FS);
        let expected = offset * f64::from(FS);
        let measured = analysis.first_beat_sample as f64;
        let wrapped = (measured - expected).rem_euclid(period_samples);
        let distance = wrapped.min(period_samples - wrapped);
        assert!(
            distance < 2_048.0,
            "{bpm} BPM : premier beat à {measured}, attendu ≈ {expected} \
             (distance modulaire {distance:.0} samples)"
        );
    }
}

#[test]
fn le_silence_n_a_pas_de_tempo() {
    let silence = vec![0.0f32; 48_000 * 20];
    assert!(analyze_track(&silence, FS).is_none());
}

#[test]
fn piste_trop_courte_refusee() {
    let short = click_track(120.0, 1.0, 0.0);
    assert!(analyze_track(&short, FS).is_none());
}
