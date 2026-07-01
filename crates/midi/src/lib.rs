//! I/O MIDI (midir) et moteur de mapping générique : `événement MIDI →
//! mapping::Action` entrant, `StateChange → message MIDI` sortant (feedback
//! LED). Aucune dépendance Bevy.
//!
//! Contraintes clés (specs §5.1) :
//! - **chemin court** pour les contrôles critiques : jogs, faders et
//!   crossfader partent du thread MIDI directement vers le thread audio
//!   (commandes `engine`), sans passer par le scheduler Bevy — la latence
//!   d'une frame est inacceptable pour le scratch. Bevy reçoit une copie
//!   pour l'affichage ;
//! - **hot-plug** : détection connexion/déconnexion, reconnexion
//!   automatique, jamais de crash au débranchement.

/// Événement MIDI brut horodaté (µs, horloge midir).
#[derive(Debug, Clone)]
pub struct RawMidiEvent {
    pub timestamp_us: u64,
    pub bytes: Vec<u8>,
}

/// Liste les ports MIDI d'entrée visibles. Utilisé par l'app (détection du
/// contrôleur via `device_match`) et par l'outil `midi-probe`.
pub fn list_input_ports() -> Result<Vec<String>, midir::InitError> {
    let input = midir::MidiInput::new("ober")?;
    Ok(input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect())
}
