//! État audio → UI, publié par triple buffer (specs §2.3). Instantanés
//! copiés, jamais de mémoire partagée mutable avec le callback.

/// État d'un deck tel que vu par le thread audio.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeckSnapshot {
    pub playing: bool,
    /// Pré-écoute casque active (état pour l'UI et les LEDs, M5).
    pub cue: bool,
    /// Position de lecture dans la piste, en samples (48 kHz).
    pub position_samples: u64,
    /// Longueur de la piste chargée, en samples (0 = pas de piste).
    pub track_frames: u64,
    /// Vitesse de lecture courante (1.0 = nominale, 0.0 = à l'arrêt). Sert à
    /// l'extrapolation de la position affichée entre snapshots (specs §6.1).
    pub speed: f64,
    /// Niveaux du deck après gain et crossfader, par canal, sur le dernier bloc.
    pub rms: [f32; 2],
    pub peak: [f32; 2],
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EngineSnapshot {
    pub decks: [DeckSnapshot; 2],
    pub master_rms: [f32; 2],
    pub master_peak: [f32; 2],
    /// Compteur cumulé : erreurs de stream cpal + callbacks ayant dépassé
    /// leur budget temps (specs §3.6). Affiché dans la barre d'état.
    pub underruns: u64,
    /// Fraction du budget temps du callback consommée, lissée `[0, 1+]` ;
    /// l'objectif est < 0,20 (specs §3.6).
    pub callback_load: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_par_defaut_silencieux_et_arrete() {
        let s = EngineSnapshot::default();
        assert!(!s.decks[0].playing && !s.decks[1].playing);
        assert_eq!(s.underruns, 0);
    }
}
