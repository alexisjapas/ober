//! Thread I/O MIDI (specs §5.1) : connexion au contrôleur, hot-plug par
//! polling (midir ne notifie pas les débranchements), et bidirectionnel :
//!
//! - **entrée, chemin court** : événement traduit → `EngineCommand` poussé
//!   dans le ring SPSC dédié du moteur, directement depuis le callback
//!   midir ; copie de chaque événement vers Bevy (canal crossbeam) ;
//! - **sortie, feedback LED** (specs §5.2) : l'app relaie les snapshots du
//!   moteur ; le moteur de feedback n'émet que les changements, à ~30 Hz,
//!   sur une connexion de sortie persistante (init LEDs à la connexion).
//!
//! L'application ne crashe jamais sur un débranchement : les connexions
//! sont fermées puis retentées au cycle de scan suivant.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use engine::{EngineCommand, EngineSnapshot};
use mapping::Mapping;

use crate::feedback::FeedbackEngine;
use crate::route::to_engine_command;
use crate::translate::{ControlEvent, MappingEngine};

/// Cadence du feedback (et réactivité de l'arrêt du thread).
const TICK: Duration = Duration::from_millis(33);
/// Scan des ports toutes les ~1,5 s (45 ticks).
const SCAN_EVERY_TICKS: u32 = 45;

/// Statut de connexion, affiché par l'UI (barre d'état).
#[derive(Debug, Clone, PartialEq)]
pub enum MidiStatus {
    Connected(String),
    Disconnected,
}

/// État partagé entre les connexions successives (le producteur SPSC et
/// l'état des toggles/shift survivent aux débranchements). Le Mutex n'est
/// pris que par le callback midir — jamais par le thread audio.
struct Route {
    engine: MappingEngine,
    commands: rtrb::Producer<EngineCommand>,
    events: crossbeam_channel::Sender<ControlEvent>,
    /// Rate of the opened output stream — scales seek/EQ conversions.
    sample_rate: u32,
}

/// Poignée du sous-système MIDI. Au drop : arrêt du thread et fermeture des
/// connexions.
pub struct MidiIo {
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl MidiIo {
    /// Démarre le thread MIDI. `commands` est le producteur du ring dédié
    /// (`EnginePorts::midi_commands`) ; `snapshots` reçoit les copies
    /// d'état relayées par l'app pour le feedback LED. `sample_rate` is the
    /// rate of the opened output stream (`StreamInfo::sample_rate`).
    pub fn spawn(
        mapping: Mapping,
        commands: rtrb::Producer<EngineCommand>,
        events: crossbeam_channel::Sender<ControlEvent>,
        status: crossbeam_channel::Sender<MidiStatus>,
        snapshots: crossbeam_channel::Receiver<EngineSnapshot>,
        sample_rate: u32,
    ) -> std::io::Result<Self> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("ober-midi".into())
            .spawn(move || {
                midi_thread(
                    &mapping,
                    commands,
                    &events,
                    &status,
                    &snapshots,
                    &shutdown_rx,
                    sample_rate,
                )
            })?;
        Ok(Self {
            shutdown: shutdown_tx,
            thread: Some(thread),
        })
    }
}

impl Drop for MidiIo {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn midi_thread(
    mapping: &Mapping,
    commands: rtrb::Producer<EngineCommand>,
    events: &crossbeam_channel::Sender<ControlEvent>,
    status: &crossbeam_channel::Sender<MidiStatus>,
    snapshots: &crossbeam_channel::Receiver<EngineSnapshot>,
    shutdown: &mpsc::Receiver<()>,
    sample_rate: u32,
) {
    let mut commands = commands;
    // Jog model parameters come from the mapping (specs §3.5); the
    // per-sample coefficients are derived here, outside the callback, with
    // the stream's rate (Rule 5).
    let _ = commands.push(EngineCommand::SetJogParams(engine::JogRuntime::new(
        crate::route::jog_params(&mapping.jog),
        sample_rate,
    )));

    let route = Arc::new(Mutex::new(Route {
        engine: MappingEngine::new(mapping),
        commands,
        events: events.clone(),
        sample_rate,
    }));
    let mut feedback = FeedbackEngine::new(mapping, sample_rate);
    let mut input: Option<(MidiInputConnection<()>, String)> = None;
    let mut output: Option<MidiOutputConnection> = None;
    let mut last_snapshot = EngineSnapshot::default();
    let mut messages: Vec<[u8; 3]> = Vec::new();
    let mut tick: u32 = 0;

    loop {
        // Scan des ports : détection du débranchement et (re)connexion.
        if tick.is_multiple_of(SCAN_EVERY_TICKS) {
            let ports = crate::list_input_ports().unwrap_or_default();

            if let Some((_, name)) = &input
                && !ports.iter().any(|p| p == name)
            {
                input = None;
                output = None;
                let _ = status.send(MidiStatus::Disconnected);
            }

            if input.is_none()
                && let Some(port_name) = ports.iter().find(|p| mapping.matches_port(p))
                && let Some(connection) = connect_input(port_name, &route)
            {
                output = connect_output(mapping);
                if let Some(out) = output.as_mut() {
                    // Init (ex. mode « full MIDI » des LEDs Hercules).
                    for (a, b, c) in &mapping.init {
                        let _ = out.send(&[*a, *b, *c]);
                    }
                }
                feedback.reset();
                input = Some((connection, port_name.clone()));
                let _ = status.send(MidiStatus::Connected(port_name.clone()));
            }
        }
        tick = tick.wrapping_add(1);

        // Dernier état publié par le moteur (relayé par l'app).
        while let Ok(snapshot) = snapshots.try_recv() {
            last_snapshot = snapshot;
        }

        // Feedback LED : uniquement les changements (specs §5.2).
        if let Some(out) = output.as_mut() {
            messages.clear();
            feedback.refresh(&last_snapshot, &mut messages);
            let failed = messages.iter().any(|msg| out.send(msg).is_err());
            if failed {
                // Sortie morte (débranchement…) : le scan suivant retentera.
                output = None;
            }
        }

        match shutdown.recv_timeout(TICK) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    drop(input);
    drop(output);
}

fn connect_input(port_name: &str, route: &Arc<Mutex<Route>>) -> Option<MidiInputConnection<()>> {
    let mut input = MidiInput::new("ober").ok()?;
    input.ignore(Ignore::None);
    let ports = input.ports();
    let port = ports
        .iter()
        .find(|p| input.port_name(p).is_ok_and(|n| n == port_name))?;

    let route = Arc::clone(route);
    input
        .connect(
            port,
            "ober-in",
            move |_timestamp, bytes, ()| {
                // Verrou jamais contendu (seul ce callback le prend) ; le
                // thread audio n'est pas concerné — il lit son ring SPSC.
                let Ok(mut route) = route.lock() else { return };
                let sample_rate = route.sample_rate;
                if let Some(event) = route.engine.translate(bytes) {
                    if let Some(command) = to_engine_command(&event, sample_rate) {
                        let _ = route.commands.push(command);
                    }
                    let _ = route.events.send(event);
                }
            },
            (),
        )
        .ok()
}

/// Ouvre le port de sortie du contrôleur (feedback LED). Best-effort :
/// sans port de sortie, les LEDs resteront muettes, l'entrée fonctionne.
fn connect_output(mapping: &Mapping) -> Option<MidiOutputConnection> {
    let output = MidiOutput::new("ober").ok()?;
    let ports = output.ports();
    let port = ports
        .iter()
        .find(|p| output.port_name(p).is_ok_and(|n| mapping.matches_port(&n)))?;
    output.connect(port, "ober-out").ok()
}
