//! Test d'intégration : rendu offline du graphe audio — mêmes structs que le
//! callback cpal, appelées hors cpal (specs §7). Poser
//! `OBER_WRITE_WAV=1` pour écrire les rendus dans `target/` et les écouter.

use std::sync::Arc;

use engine::{AudioGraph, CHANNELS, Deck, EngineCommand, PREFERRED_SAMPLE_RATE, TrackBuffer};

const BLOCK_FRAMES: usize = 256;

fn sine_track(freq: f32, seconds: f32, amplitude: f32) -> Arc<TrackBuffer> {
    let frames = (seconds * PREFERRED_SAMPLE_RATE as f32) as usize;
    let mut samples = Vec::with_capacity(frames * CHANNELS);
    for i in 0..frames {
        let s = amplitude
            * (std::f32::consts::TAU * freq * i as f32 / PREFERRED_SAMPLE_RATE as f32).sin();
        samples.push(s);
        samples.push(s);
    }
    TrackBuffer::new(samples)
}

fn render(graph: &mut AudioGraph, blocks: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; BLOCK_FRAMES * CHANNELS];
    let mut rendered = Vec::with_capacity(blocks * out.len());
    for _ in 0..blocks {
        graph.process(&mut out);
        rendered.extend_from_slice(&out);
    }
    rendered
}

fn write_wav_if_asked(name: &str, samples: &[f32]) {
    if std::env::var_os("OBER_WRITE_WAV").is_none() {
        return;
    }
    let spec = hound::WavSpec {
        channels: CHANNELS as u16,
        sample_rate: PREFERRED_SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let path = format!("../../target/{name}.wav");
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for &s in samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
    eprintln!("WAV écrit : {path}");
}

#[test]
fn mix_2_decks_au_centre() {
    let (mut graph, mut ports) = AudioGraph::new();
    // L'UI garde un clone de chaque Arc envoyé (règle de `track.rs`).
    let a = sine_track(440.0, 1.0, 0.5);
    let b = sine_track(220.0, 1.0, 0.5);

    let cmds = &mut ports.commands;
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
        .unwrap();
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::B, b.clone()))
        .unwrap();
    cmds.push(EngineCommand::SetCrossfader(0.0)).unwrap();
    cmds.push(EngineCommand::Play(Deck::A)).unwrap();
    cmds.push(EngineCommand::Play(Deck::B)).unwrap();

    let rendered = render(
        &mut graph,
        PREFERRED_SAMPLE_RATE as usize / 2 / BLOCK_FRAMES,
    );
    graph.publish_snapshot();
    write_wav_if_asked("offline_mix_2decks", &rendered);

    let peak = rendered.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.3, "signal attendu, peak = {peak}");
    assert!(peak <= 1.0, "écrêtage inattendu, peak = {peak}");

    // Deux sinus d'amplitude 0,5 × gain crossfader ≈ 0,707 : RMS combiné
    // attendu autour de 0,5 · 0,707 (≈ 0,25 par sinus, ~0,35 sommé).
    let rms = (rendered.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / rendered.len() as f64)
        .sqrt();
    assert!((0.2..0.5).contains(&rms), "rms = {rms}");

    let snapshot = ports.snapshots.read();
    assert!(snapshot.decks[0].playing && snapshot.decks[1].playing);
    assert_eq!(snapshot.underruns, 0);
    assert!(snapshot.master_rms[0] > 0.1);
}

#[test]
fn crossfader_a_gauche_coupe_le_deck_b() {
    let (mut graph, mut ports) = AudioGraph::new();
    let b = sine_track(220.0, 0.5, 0.8);
    ports
        .commands
        .push(EngineCommand::SwapTrackBuffer(Deck::B, b.clone()))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::SetCrossfader(-1.0))
        .unwrap();
    ports.commands.push(EngineCommand::Play(Deck::B)).unwrap();

    let rendered = render(&mut graph, 20);
    graph.publish_snapshot();
    let peak = rendered.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak < 1e-6,
        "B doit être inaudible à fond à gauche, peak = {peak}"
    );

    // Le deck avance quand même : le crossfader coupe le gain, pas la lecture.
    let snapshot = ports.snapshots.read();
    assert!(snapshot.decks[1].position_samples > 0);
}

#[test]
fn fin_de_piste_arrete_le_deck() {
    let (mut graph, mut ports) = AudioGraph::new();
    let short = sine_track(440.0, 0.01, 0.5); // 480 frames
    let frames = short.frames() as u64;
    ports
        .commands
        .push(EngineCommand::SwapTrackBuffer(Deck::A, short.clone()))
        .unwrap();
    ports.commands.push(EngineCommand::Play(Deck::A)).unwrap();

    let _ = render(&mut graph, 4); // 1024 frames > 480
    graph.publish_snapshot();

    let snapshot = ports.snapshots.read();
    assert!(!snapshot.decks[0].playing);
    assert_eq!(snapshot.decks[0].position_samples, frames);
}

#[test]
fn swap_renvoie_l_ancien_buffer_par_le_canal_de_recuperation() {
    let (mut graph, mut ports) = AudioGraph::new();
    let first = sine_track(440.0, 0.05, 0.5);
    let second = sine_track(330.0, 0.05, 0.5);

    ports
        .commands
        .push(EngineCommand::SwapTrackBuffer(Deck::A, first.clone()))
        .unwrap();
    let mut out = vec![0.0f32; BLOCK_FRAMES * CHANNELS];
    graph.process(&mut out);

    ports
        .commands
        .push(EngineCommand::SwapTrackBuffer(Deck::A, second.clone()))
        .unwrap();
    graph.process(&mut out);

    // L'ancien buffer est ressorti côté worker, pas droppé dans le callback.
    let reclaimed = ports.reclaim.pop().expect("un buffer à récupérer");
    assert!(Arc::ptr_eq(&reclaimed, &first));

    // Seek clampé à la fin de piste.
    ports
        .commands
        .push(EngineCommand::SeekSamples(Deck::A, u64::MAX))
        .unwrap();
    graph.process(&mut out);
    graph.publish_snapshot();
    let snapshot = ports.snapshots.read();
    assert_eq!(snapshot.decks[0].position_samples, second.frames() as u64);
}

#[test]
fn le_tap_recoit_le_mix_post_master() {
    let (mut graph, mut ports) = AudioGraph::new();
    let a = sine_track(440.0, 0.5, 0.5);
    ports
        .commands
        .push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
        .unwrap();
    ports.commands.push(EngineCommand::Play(Deck::A)).unwrap();

    let mut out = vec![0.0f32; BLOCK_FRAMES * CHANNELS];
    graph.process(&mut out);

    let mut tapped = Vec::new();
    while let Ok(s) = ports.tap.pop() {
        tapped.push(s);
    }
    assert_eq!(tapped.len(), out.len());
    assert_eq!(tapped, out);
}
