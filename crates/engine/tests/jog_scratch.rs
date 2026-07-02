//! Tests d'intégration M4 : modèle scratch/bend à travers le graphe complet
//! (rendu offline). Le réglage fin se fait à l'oreille sur le matériel —
//! ici on vérifie la mécanique : suivi de vélocité, absence de sauts,
//! rampe de relâchement, scratch arrière borné au début de piste.

use engine::{AudioGraph, CHANNELS, Deck, EngineCommand, EnginePorts, SAMPLE_RATE, TrackBuffer};

const BLOCK: usize = 256;

fn setup(seconds: f32) -> (AudioGraph, EnginePorts) {
    let (mut graph, mut ports) = AudioGraph::new();
    let frames = (seconds * SAMPLE_RATE as f32) as usize;
    let mut samples = Vec::with_capacity(frames * CHANNELS);
    for i in 0..frames {
        let s = 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / SAMPLE_RATE as f32).sin();
        samples.push(s);
        samples.push(s);
    }
    ports
        .commands
        .push(EngineCommand::SwapTrackBuffer(
            Deck::A,
            TrackBuffer::new(samples),
        ))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::SetCrossfader(-1.0))
        .unwrap();
    let mut out = [0.0f32; BLOCK * CHANNELS];
    graph.process(&mut out);
    (graph, ports)
}

/// Rend `blocks` blocs en injectant `ticks_per_block` ticks avant chacun.
/// Retourne les positions publiées après chaque bloc.
fn run_with_ticks(
    graph: &mut AudioGraph,
    ports: &mut EnginePorts,
    blocks: usize,
    ticks_per_block: i32,
) -> Vec<u64> {
    let mut out = [0.0f32; BLOCK * CHANNELS];
    let mut positions = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        if ticks_per_block != 0 {
            ports
                .commands
                .push(EngineCommand::JogTicks(Deck::A, ticks_per_block))
                .unwrap();
        }
        graph.process(&mut out);
        graph.publish_snapshot();
        positions.push(ports.snapshots.read().decks[0].position_samples);
    }
    positions
}

#[test]
fn scratch_suit_la_rotation_sans_escalier() {
    let (mut graph, mut ports) = setup(10.0);
    ports
        .commands
        .push(EngineCommand::SeekSamples(Deck::A, 96_000))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::JogTouch(Deck::A, true))
        .unwrap();

    // Rotation à 2× la vitesse nominale : ticks/s = 2 × 400 = 800, soit
    // ~4,27 ticks par bloc de 256 frames. Le matériel livre les ticks au
    // fil de l'eau : on injecte 4 par bloc (5 chaque 4ᵉ), comme le fait le
    // drain des commandes en conditions réelles.
    let mut out = [0.0f32; BLOCK * CHANNELS];
    let mut positions = Vec::new();
    for i in 0..200 {
        let ticks = if i % 4 == 3 { 5 } else { 4 };
        ports
            .commands
            .push(EngineCommand::JogTicks(Deck::A, ticks))
            .unwrap();
        graph.process(&mut out);
        graph.publish_snapshot();
        positions.push(ports.snapshots.read().decks[0].position_samples as f64);
    }

    // Après convergence (~50 blocs), la vitesse moyenne doit être ≈ 2×.
    let settled = &positions[100..];
    let avg_speed =
        (settled.last().unwrap() - settled.first().unwrap()) / ((settled.len() - 1) * BLOCK) as f64;
    assert!(
        (avg_speed - 2.0).abs() < 0.15,
        "vitesse moyenne de scratch = {avg_speed}, attendu ≈ 2.0"
    );

    // Pas de son « escalier » : les avancées par bloc ne sautent jamais
    // brutalement (lissage passe-bas τ≈5 ms).
    let mut prev_delta: Option<f64> = None;
    for pair in settled.windows(2) {
        let delta = pair[1] - pair[0];
        if let Some(prev) = prev_delta {
            assert!(
                (delta - prev).abs() < 0.8 * BLOCK as f64,
                "saut de vitesse entre blocs : {prev} → {delta}"
            );
        }
        prev_delta = Some(delta);
    }
}

#[test]
fn relachement_d_un_deck_a_l_arret_s_arrete_en_douceur() {
    let (mut graph, mut ports) = setup(10.0);
    ports
        .commands
        .push(EngineCommand::SeekSamples(Deck::A, 48_000))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::JogTouch(Deck::A, true))
        .unwrap();

    // Scratch avant, puis relâchement.
    let _ = run_with_ticks(&mut graph, &mut ports, 60, 8);
    ports
        .commands
        .push(EngineCommand::JogTouch(Deck::A, false))
        .unwrap();

    // Rampe de 100 ms = 4 800 frames ≈ 19 blocs : le deck (à l'arrêt)
    // décélère progressivement puis se fige.
    let positions = run_with_ticks(&mut graph, &mut ports, 40, 0);
    let during_ramp = positions[2] - positions[0];
    assert!(during_ramp > 0, "la piste glisse encore pendant la rampe");
    let after = positions[38] - positions[30];
    assert_eq!(after, 0, "après la rampe, deck à l'arrêt figé");

    graph.publish_snapshot();
    assert!(!ports.snapshots.read().decks[0].playing);
}

#[test]
fn bend_accelere_puis_revient_a_la_nominale() {
    let (mut graph, mut ports) = setup(20.0);
    ports.commands.push(EngineCommand::Play(Deck::A)).unwrap();

    // Lecture nominale de référence.
    let positions = run_with_ticks(&mut graph, &mut ports, 40, 0);
    let nominal_delta = (positions[39] - positions[19]) as f64 / 20.0;
    assert!((nominal_delta - BLOCK as f64).abs() < 1.0);

    // Bord du jog tourné (sans touch) : ~2 ticks/bloc ≈ vitesse nominale
    // → offset attendu ≈ bend_sensitivity (0,3).
    let positions = run_with_ticks(&mut graph, &mut ports, 120, 2);
    let bent_delta = (positions[119] - positions[79]) as f64 / 40.0;
    let bent_speed = bent_delta / BLOCK as f64;
    assert!(
        bent_speed > 1.15 && bent_speed < 1.45,
        "vitesse en bend = {bent_speed}, attendu ≈ 1.3"
    );

    // Rotation stoppée : retour progressif à la nominale (τ = 150 ms).
    let positions = run_with_ticks(&mut graph, &mut ports, 200, 0);
    let recovered = (positions[199] - positions[159]) as f64 / 40.0 / BLOCK as f64;
    assert!(
        (recovered - 1.0).abs() < 0.03,
        "vitesse après retour = {recovered}, attendu ≈ 1.0"
    );
}

#[test]
fn scratch_arriere_borne_au_debut_de_piste() {
    let (mut graph, mut ports) = setup(5.0);
    ports
        .commands
        .push(EngineCommand::SeekSamples(Deck::A, 2_000))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::JogTouch(Deck::A, true))
        .unwrap();

    // Grand coup en arrière.
    let positions = run_with_ticks(&mut graph, &mut ports, 100, -40);
    let last = *positions.last().unwrap();
    assert_eq!(last, 0, "position clampée au début de piste : {last}");
}
