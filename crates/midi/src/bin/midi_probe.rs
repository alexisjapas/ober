//! midi-probe : logge en hexadécimal tous les messages MIDI entrants de tous
//! les ports. Outil de rétro-ingénierie contrôleur (specs §5.3), utile pour
//! valider chaque contrôle de l'Inpulse 200 MK2 et les futurs contrôleurs.
//!
//! Usage : `cargo run -p midi --bin midi-probe`

use std::io::stdin;

use midir::{Ignore, MidiInput};

fn main() -> anyhow::Result<()> {
    let n_ports = new_input()?.ports().len();
    if n_ports == 0 {
        println!("Aucun port MIDI d'entrée détecté.");
        return Ok(());
    }

    let mut connections = Vec::new();
    for index in 0..n_ports {
        let input = new_input()?;
        let ports = input.ports();
        let Some(port) = ports.get(index) else {
            continue; // port disparu entre l'énumération et la connexion
        };
        let name = input.port_name(port)?;
        println!("Écoute de « {name} »");
        let label = name.clone();
        let connection = input
            .connect(
                port,
                "midi-probe",
                move |timestamp_us, message, ()| {
                    let hex: Vec<String> = message.iter().map(|b| format!("{b:02X}")).collect();
                    println!("[{timestamp_us:>12} µs] {label:<40} {}", hex.join(" "));
                },
                (),
            )
            .map_err(|e| anyhow::anyhow!("connexion au port « {name} » : {e}"))?;
        connections.push(connection);
    }

    println!("\nAppuyez sur Entrée pour quitter.");
    let mut line = String::new();
    stdin().read_line(&mut line)?;
    drop(connections);
    Ok(())
}

fn new_input() -> anyhow::Result<MidiInput> {
    let mut input = MidiInput::new("midi-probe")?;
    input.ignore(Ignore::None);
    Ok(input)
}
