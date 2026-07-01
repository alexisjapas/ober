//! État audio → UI, publié par triple buffer (specs §2.3). Instantanés
//! copiés, jamais de mémoire partagée mutable avec le callback.

/// État d'un deck tel que vu par le thread audio.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeckSnapshot {
    pub playing: bool,
    /// Position de lecture dans la piste, en samples (48 kHz).
    pub position_samples: u64,
    /// Vitesse de lecture courante (1.0 = nominale). Sert à l'extrapolation
    /// de la position affichée entre deux snapshots (specs §6.1).
    pub speed: f64,
    pub rms: [f32; 2],
    pub peak: [f32; 2],
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EngineSnapshot {
    pub decks: [DeckSnapshot; 2],
    pub master_rms: [f32; 2],
    pub master_peak: [f32; 2],
    /// Compteur cumulé d'underruns (specs §3.6), affiché dans la barre d'état.
    pub underruns: u64,
    /// Fraction du budget temps du callback consommée `[0, 1]` ; budget < 20 %.
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
