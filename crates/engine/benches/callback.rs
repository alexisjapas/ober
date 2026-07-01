//! Bench criterion : coût du chemin critique du callback pour 2 decks actifs
//! à 128 frames (specs §7) — chaîne M2 complète : varispeed Hermite, EQ
//! 3 bandes, cue, limiteur, sortie 4 canaux. Budget : < 20 % du temps réel,
//! soit < 533 µs pour un bloc de 128 frames à 48 kHz — cible < ~107 µs.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use engine::dsp::{EqBand, eq_coeffs};
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
    // Cas réaliste chargé : 4 canaux (master + casque), varispeed hors
    // nominal, EQ non transparents, un deck en pré-écoute.
    graph.set_output_channels(4);
    let a = sine_track(440.0, 30.0);
    let b = sine_track(220.0, 30.0);
    let fs = f64::from(SAMPLE_RATE);
    let cmds = &mut ports.commands;
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::A, a.clone()))
        .unwrap();
    cmds.push(EngineCommand::SwapTrackBuffer(Deck::B, b.clone()))
        .unwrap();
    for deck in Deck::ALL {
        cmds.push(EngineCommand::SetPitch(deck, 1.043)).unwrap();
        for band in EqBand::ALL {
            cmds.push(EngineCommand::SetEq(deck, band, eq_coeffs(band, -6.0, fs)))
                .unwrap();
        }
    }
    cmds.push(EngineCommand::SetCueEnabled(Deck::B, true))
        .unwrap();
    cmds.push(EngineCommand::Play(Deck::A)).unwrap();
    cmds.push(EngineCommand::Play(Deck::B)).unwrap();

    let mut out = vec![0.0f32; BLOCK_FRAMES * 4];
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
