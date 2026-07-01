//! Bench criterion : coût du chemin critique du callback pour 2 decks actifs
//! à 128 frames (specs §7). Budget : < 20 % du temps réel, soit < 533 µs
//! pour un bloc de 128 frames à 48 kHz — la cible est donc < ~107 µs.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use engine::{AudioGraph, CHANNELS, Deck, EngineCommand, SAMPLE_RATE, TrackBuffer};

const BLOCK_FRAMES: usize = 128;

fn sine_track(freq: f32, seconds: f32) -> Arc<TrackBuffer> {
    let frames = (seconds * SAMPLE_RATE as f32) as usize;
    let mut samples = Vec::with_capacity(frames * CHANNELS);
    for i in 0..frames {
        let s = 0.5 * (std::f32::consts::TAU * freq * i as f32 / SAMPLE_RATE as f32).sin();
        samples.push(s);
        samples.push(s);
    }
    TrackBuffer::new(samples)
}

fn callback_2_decks(c: &mut Criterion) {
    let (mut graph, mut ports) = AudioGraph::new();
    let a = sine_track(440.0, 30.0);
    let b = sine_track(220.0, 30.0);
    let cmds = &mut ports.commands;
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
        .unwrap();
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::B, b.clone()))
        .unwrap();
    cmds.push(EngineCommand::Play(Deck::A)).unwrap();
    cmds.push(EngineCommand::Play(Deck::B)).unwrap();

    let mut out = vec![0.0f32; BLOCK_FRAMES * CHANNELS];
    // Premier bloc : draine les commandes d'installation.
    graph.process(&mut out);

    c.bench_function("process_2decks_128frames", |bench| {
        bench.iter(|| {
            // Re-seek à chaque itération pour que les decks ne s'arrêtent
            // jamais en fin de piste pendant la mesure (2 commandes drainées
            // par process, comme en conditions réelles).
            let _ = ports.commands.push(EngineCommand::SeekSamples(Deck::A, 0));
            let _ = ports.commands.push(EngineCommand::SeekSamples(Deck::B, 0));
            graph.process(black_box(&mut out));
        });
    });
}

criterion_group!(benches, callback_2_decks);
criterion_main!(benches);
