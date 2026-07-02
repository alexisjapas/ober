//! Thread I/O MIDI (specs §5.1) : connexion au contrôleur, hot-plug par
//! polling (midir ne notifie pas les débranchements), envoi des messages
//! d'init (mode « full MIDI » des LEDs Hercules), et routage :
//!
//! - **chemin court** : événement traduit → `EngineCommand` poussé dans le
//!   ring SPSC dédié du moteur, directement depuis le callback midir ;
//! - **copie UI** : chaque événement part aussi vers Bevy (canal crossbeam)
//!   pour l'affichage.
//!
//! L'application ne crashe jamais sur un débranchement : la connexion est
//! simplement fermée puis retentée à chaque cycle de polling.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput};

use engine::EngineCommand;
use mapping::Mapping;

use crate::route::to_engine_command;
use crate::translate::{ControlEvent, MappingEngine};

const POLL_INTERVAL: Duration = Duration::from_millis(1500);

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
}

/// Poignée du sous-système MIDI. Au drop : arrêt du thread et fermeture de
/// la connexion.
pub struct MidiIo {
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl MidiIo {
    /// Démarre le thread MIDI. `commands` est le producteur du ring dédié
    /// (`EnginePorts::midi_commands`).
    pub fn spawn(
        mapping: Mapping,
        commands: rtrb::Producer<EngineCommand>,
        events: crossbeam_channel::Sender<ControlEvent>,
        status: crossbeam_channel::Sender<MidiStatus>,
    ) -> std::io::Result<Self> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("ober-midi".into())
            .spawn(move || midi_thread(&mapping, commands, &events, &status, &shutdown_rx))?;
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

fn midi_thread(
    mapping: &Mapping,
    commands: rtrb::Producer<EngineCommand>,
    events: &crossbeam_channel::Sender<ControlEvent>,
    status: &crossbeam_channel::Sender<MidiStatus>,
    shutdown: &mpsc::Receiver<()>,
) {
    let route = Arc::new(Mutex::new(Route {
        engine: MappingEngine::new(mapping),
        commands,
        events: events.clone(),
    }));
    let mut connection: Option<(MidiInputConnection<()>, String)> = None;

    loop {
        let ports = crate::list_input_ports().unwrap_or_default();

        // Débranchement : le port connecté a disparu.
        if let Some((_, name)) = &connection
            && !ports.iter().any(|p| p == name)
        {
            connection = None;
            let _ = status.send(MidiStatus::Disconnected);
        }

        // (Re)connexion au premier port qui matche le mapping.
        if connection.is_none()
            && let Some(port_name) = ports.iter().find(|p| mapping.matches_port(p))
            && let Some(conn) = connect(port_name, &route)
        {
            send_init(mapping);
            connection = Some((conn, port_name.clone()));
            let _ = status.send(MidiStatus::Connected(port_name.clone()));
        }

        match shutdown.recv_timeout(POLL_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    drop(connection);
}

fn connect(port_name: &str, route: &Arc<Mutex<Route>>) -> Option<MidiInputConnection<()>> {
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
                if let Some(event) = route.engine.translate(bytes) {
                    if let Some(command) = to_engine_command(&event) {
                        let _ = route.commands.push(command);
                    }
                    let _ = route.events.send(event);
                }
            },
            (),
        )
        .ok()
}

/// Envoie les messages d'init du mapping (ex. activation des LEDs Hercules)
/// sur le port de sortie du contrôleur. Best-effort : sans port de sortie,
/// tant pis — le feedback complet arrive au M5.
fn send_init(mapping: &Mapping) {
    if mapping.init.is_empty() {
        return;
    }
    let Ok(output) = MidiOutput::new("ober") else {
        return;
    };
    let ports = output.ports();
    let Some(port) = ports
        .iter()
        .find(|p| output.port_name(p).is_ok_and(|n| mapping.matches_port(&n)))
    else {
        return;
    };
    if let Ok(mut conn) = output.connect(port, "ober-out") {
        for (a, b, c) in &mapping.init {
            let _ = conn.send(&[*a, *b, *c]);
        }
    }
}
