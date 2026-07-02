//! Sémantique vinyle du bouton cue (résolue dans le moteur, qui connaît son
//! état — cf. `EngineCommand::CuePress`), testée en rendu offline.

use std::sync::Arc;

use engine::{AudioGraph, CHANNELS, Deck, EngineCommand, EnginePorts, SAMPLE_RATE, TrackBuffer};

const BLOCK: usize = 256;

fn setup() -> (AudioGraph, EnginePorts, Arc<TrackBuffer>) {
    let (mut graph, mut ports) = AudioGraph::new();
    let frames = SAMPLE_RATE as usize; // 1 s
    let track = TrackBuffer::new(vec![0.1f32; frames * CHANNELS]);
    ports
        .commands
        .push(EngineCommand::SwapTrackBuffer(Deck::A, track.clone()))
        .unwrap();
    let mut out = [0.0f32; BLOCK * CHANNELS];
    graph.process(&mut out);
    (graph, ports, track)
}

fn run(graph: &mut AudioGraph, blocks: usize) {
    let mut out = [0.0f32; BLOCK * CHANNELS];
    for _ in 0..blocks {
        graph.process(&mut out);
    }
}

#[test]
fn cue_a_l_arret_pose_le_point_puis_previsualise() {
    let (mut graph, mut ports, _track) = setup();

    // Avance à ~0,5 s à l'arrêt, puis pose le cue.
    ports
        .commands
        .push(EngineCommand::SeekSamples(Deck::A, 24_000))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::CuePress(Deck::A))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::CueRelease(Deck::A))
        .unwrap();
    run(&mut graph, 1);
    graph.publish_snapshot();
    let snap = *ports.snapshots.read();
    assert_eq!(snap.decks[0].cue_point_samples, 24_000);
    assert!(!snap.decks[0].playing);

    // Sur le cue : press = pré-écoute (lecture), release = retour au cue.
    ports
        .commands
        .push(EngineCommand::CuePress(Deck::A))
        .unwrap();
    run(&mut graph, 4);
    graph.publish_snapshot();
    let snap = *ports.snapshots.read();
    assert!(snap.decks[0].playing, "pré-écoute pendant la tenue");
    assert!(snap.decks[0].position_samples > 24_000);

    ports
        .commands
        .push(EngineCommand::CueRelease(Deck::A))
        .unwrap();
    run(&mut graph, 1);
    graph.publish_snapshot();
    let snap = *ports.snapshots.read();
    assert!(!snap.decks[0].playing);
    assert_eq!(
        snap.decks[0].position_samples, 24_000,
        "retour au point cue"
    );
}

#[test]
fn cue_en_lecture_stoppe_et_revient_au_point() {
    let (mut graph, mut ports, _track) = setup();

    ports
        .commands
        .push(EngineCommand::SeekSamples(Deck::A, 12_000))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::CuePress(Deck::A))
        .unwrap(); // pose à 12 000
    ports
        .commands
        .push(EngineCommand::CueRelease(Deck::A))
        .unwrap();
    ports.commands.push(EngineCommand::Play(Deck::A)).unwrap();
    run(&mut graph, 8); // lit ~2 048 frames

    ports
        .commands
        .push(EngineCommand::CuePress(Deck::A))
        .unwrap();
    ports
        .commands
        .push(EngineCommand::CueRelease(Deck::A))
        .unwrap();
    run(&mut graph, 1);
    graph.publish_snapshot();
    let snap = *ports.snapshots.read();
    assert!(!snap.decks[0].playing, "cue en lecture = stop");
    assert_eq!(
        snap.decks[0].position_samples, 12_000,
        "retour au point cue"
    );
}

#[test]
fn play_pendant_la_previsualisation_continue_la_lecture() {
    let (mut graph, mut ports, _track) = setup();

    // Pré-écoute depuis le début de piste (cue = 0 par défaut).
    ports
        .commands
        .push(EngineCommand::CuePress(Deck::A))
        .unwrap();
    run(&mut graph, 2);
    // Play pendant la tenue : bascule en lecture normale…
    ports.commands.push(EngineCommand::Play(Deck::A)).unwrap();
    // …le relâchement du cue ne doit plus rien interrompre.
    ports
        .commands
        .push(EngineCommand::CueRelease(Deck::A))
        .unwrap();
    run(&mut graph, 2);
    graph.publish_snapshot();
    let snap = *ports.snapshots.read();
    assert!(snap.decks[0].playing, "la lecture continue après release");
    assert!(snap.decks[0].position_samples > 512);
}
