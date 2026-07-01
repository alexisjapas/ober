//! Tests d'intégration M2 : varispeed, EQ, limiteur et routage cue 4 canaux,
//! en rendu offline (mêmes structs que le callback cpal, specs §7).

use std::sync::Arc;

use engine::dsp::{EqBand, eq_coeffs};
use engine::{AudioGraph, CHANNELS, Deck, EngineCommand, SAMPLE_RATE, TrackBuffer};

const BLOCK_FRAMES: usize = 256;

fn sine_track(freq: f32, seconds: f32, amplitude: f32) -> Arc<TrackBuffer> {
    let frames = (seconds * SAMPLE_RATE as f32) as usize;
    let mut samples = Vec::with_capacity(frames * CHANNELS);
    for i in 0..frames {
        let s = amplitude * (std::f32::consts::TAU * freq * i as f32 / SAMPLE_RATE as f32).sin();
        samples.push(s);
        samples.push(s);
    }
    TrackBuffer::new(samples)
}

/// Rendu de `blocks` blocs avec `channels` canaux de sortie.
fn render(graph: &mut AudioGraph, blocks: usize, channels: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; BLOCK_FRAMES * channels];
    let mut rendered = Vec::with_capacity(blocks * out.len());
    for _ in 0..blocks {
        graph.process(&mut out);
        rendered.extend_from_slice(&out);
    }
    rendered
}

/// Fréquence estimée par passages par zéro montants sur un canal.
fn estimate_freq(interleaved: &[f32], channels: usize, channel: usize) -> f64 {
    let mono: Vec<f32> = interleaved
        .iter()
        .skip(channel)
        .step_by(channels)
        .copied()
        .collect();
    let crossings = mono
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    crossings as f64 / (mono.len() as f64 / f64::from(SAMPLE_RATE))
}

fn rms(interleaved: &[f32]) -> f64 {
    (interleaved
        .iter()
        .map(|s| f64::from(*s).powi(2))
        .sum::<f64>()
        / interleaved.len() as f64)
        .sqrt()
}

#[test]
fn varispeed_transpose_la_frequence() {
    let (mut graph, mut ports) = AudioGraph::new();
    let a = sine_track(440.0, 3.0, 0.5);
    let cmds = &mut ports.commands;
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
        .unwrap();
    cmds.push(EngineCommand::SetCrossfader(-1.0)).unwrap();
    cmds.push(EngineCommand::SetPitch(Deck::A, 1.10)).unwrap();
    cmds.push(EngineCommand::Play(Deck::A)).unwrap();

    let blocks = SAMPLE_RATE as usize / BLOCK_FRAMES; // ~1 s
    let rendered = render(&mut graph, blocks, 2);
    graph.publish_snapshot();

    let freq = estimate_freq(&rendered, 2, 0);
    assert!(
        (freq - 484.0).abs() < 5.0,
        "440 Hz à +10 % devrait donner ≈ 484 Hz, mesuré {freq:.1} Hz"
    );

    // La position avance à la vitesse demandée (comportement vinyle).
    let snapshot = ports.snapshots.read();
    let expected = (blocks * BLOCK_FRAMES) as f64 * 1.10;
    let position = snapshot.decks[0].position_samples as f64;
    assert!(
        (position - expected).abs() < BLOCK_FRAMES as f64,
        "position {position}, attendu ≈ {expected}"
    );
    assert!((snapshot.decks[0].speed - 1.10).abs() < 1e-9);
}

#[test]
fn eq_kill_des_basses_suit_la_reponse_theorique() {
    let render_60hz = |gain_db: Option<f64>| -> f64 {
        let (mut graph, mut ports) = AudioGraph::new();
        let a = sine_track(60.0, 2.0, 0.5);
        let cmds = &mut ports.commands;
        cmds.push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
            .unwrap();
        cmds.push(EngineCommand::SetCrossfader(-1.0)).unwrap();
        if let Some(db) = gain_db {
            cmds.push(EngineCommand::SetEq(
                Deck::A,
                EqBand::Low,
                eq_coeffs(EqBand::Low, db, f64::from(SAMPLE_RATE)),
            ))
            .unwrap();
        }
        cmds.push(EngineCommand::Play(Deck::A)).unwrap();
        let rendered = render(&mut graph, SAMPLE_RATE as usize / BLOCK_FRAMES, 2);
        // Ignore le transitoire d'installation du filtre.
        rms(&rendered[9_600..])
    };

    let flat = render_60hz(None);
    let killed = render_60hz(Some(-26.0));
    let measured_db = 20.0 * (killed / flat).log10();
    let expected_db = 20.0
        * eq_coeffs(EqBand::Low, -26.0, f64::from(SAMPLE_RATE))
            .magnitude_at(60.0, f64::from(SAMPLE_RATE))
            .log10();
    assert!(
        (measured_db - expected_db).abs() < 1.0,
        "atténuation mesurée {measured_db:.1} dB, théorique {expected_db:.1} dB"
    );
    assert!(
        measured_db < -18.0,
        "le kill doit être franc : {measured_db:.1} dB"
    );
}

#[test]
fn le_limiteur_borne_le_master() {
    let (mut graph, mut ports) = AudioGraph::new();
    let a = sine_track(440.0, 1.0, 0.9);
    let cmds = &mut ports.commands;
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
        .unwrap();
    cmds.push(EngineCommand::SetCrossfader(-1.0)).unwrap();
    cmds.push(EngineCommand::SetMasterGain(2.0)).unwrap();
    cmds.push(EngineCommand::Play(Deck::A)).unwrap();

    let rendered = render(&mut graph, 40, 2);
    let peak = rendered.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak < 1.0,
        "le soft-clip doit borner sous ±1, peak = {peak}"
    );
    assert!(
        peak > 0.9,
        "à gain 2, le signal doit être poussé dans la saturation, peak = {peak}"
    );
}

#[test]
fn cue_4_canaux_independant_du_crossfader() {
    let (mut graph, mut ports) = AudioGraph::new();
    graph.set_output_channels(4);

    let a = sine_track(440.0, 1.0, 0.5);
    let cmds = &mut ports.commands;
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
        .unwrap();
    // Crossfader à fond côté B : le deck A est inaudible sur le master…
    cmds.push(EngineCommand::SetCrossfader(1.0)).unwrap();
    // … mais audible au casque, cue activé et mix côté cue.
    cmds.push(EngineCommand::SetCueEnabled(Deck::A, true))
        .unwrap();
    cmds.push(EngineCommand::SetCueMix(0.0)).unwrap();
    cmds.push(EngineCommand::SetHeadphoneGain(1.0)).unwrap();
    cmds.push(EngineCommand::Play(Deck::A)).unwrap();

    let rendered = render(&mut graph, 20, 4);
    graph.publish_snapshot();

    let peak_channel = |ch: usize| -> f32 {
        rendered
            .iter()
            .skip(ch)
            .step_by(4)
            .fold(0.0f32, |m, s| m.max(s.abs()))
    };
    assert!(peak_channel(0) < 1e-6, "master gauche doit être muet");
    assert!(peak_channel(1) < 1e-6, "master droit doit être muet");
    assert!(peak_channel(2) > 0.3, "casque gauche doit entendre le cue");
    assert!(peak_channel(3) > 0.3, "casque droit doit entendre le cue");

    let freq = estimate_freq(&rendered, 4, 2);
    assert!(
        (freq - 440.0).abs() < 5.0,
        "cue = deck A, mesuré {freq:.1} Hz"
    );

    let snapshot = ports.snapshots.read();
    assert!(snapshot.decks[0].cue);

    // Balance à fond master : le casque suit le master (muet ici).
    ports.commands.push(EngineCommand::SetCueMix(1.0)).unwrap();
    let rendered = render(&mut graph, 20, 4);
    let hp_peak = rendered
        .iter()
        .skip(2)
        .step_by(4)
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        hp_peak < 1e-6,
        "mix casque à fond master (muet) : casque muet, peak = {hp_peak}"
    );
}
