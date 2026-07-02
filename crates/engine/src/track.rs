//! Buffer de piste décodée, partagé entre l'UI et le thread audio.
//!
//! L'UI conserve un clone de chaque `Arc<TrackBuffer>` envoyé au moteur :
//! ainsi un drop côté callback (canal de récupération plein, cas dégradé) ne
//! fait que décrémenter le compteur atomique, jamais désallouer.

use std::fmt;
use std::sync::Arc;

use crate::CHANNELS;

pub struct TrackBuffer {
    /// Interleaved stereo f32 at the engine's stream rate (`decode` output —
    /// the buffer itself is rate-agnostic, positions are in frames).
    samples: Vec<f32>,
}

impl TrackBuffer {
    /// Construit une piste immuable. Une longueur non multiple du nombre de
    /// canaux est tronquée à la frame complète précédente.
    pub fn new(mut samples: Vec<f32>) -> Arc<Self> {
        let complete = samples.len() - samples.len() % CHANNELS;
        samples.truncate(complete);
        Arc::new(Self { samples })
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / CHANNELS
    }

    /// Frame stéréo (gauche, droite). `idx` doit être < `frames()`.
    #[inline]
    pub fn frame(&self, idx: usize) -> (f32, f32) {
        let i = idx * CHANNELS;
        (self.samples[i], self.samples[i + 1])
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
}

impl fmt::Debug for TrackBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackBuffer")
            .field("frames", &self.frames())
            .finish()
    }
}
