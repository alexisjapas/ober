//! EQ 3 bandes par deck (specs §3.3) : low-shelf ~250 Hz, peak ~1 kHz,
//! high-shelf ~2,5 kHz. Gains −26 dB → +6 dB (le kill à −∞ optionnel arrive
//! avec le mapping M3).

use super::biquad::{self, BiquadCoeffs, BiquadState};

pub const LOW_SHELF_HZ: f64 = 250.0;
pub const PEAK_HZ: f64 = 1_000.0;
pub const PEAK_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
pub const HIGH_SHELF_HZ: f64 = 2_500.0;

pub const EQ_MIN_DB: f64 = -26.0;
pub const EQ_MAX_DB: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqBand {
    Low,
    Mid,
    High,
}

impl EqBand {
    pub const ALL: [EqBand; 3] = [EqBand::Low, EqBand::Mid, EqBand::High];

    pub fn index(self) -> usize {
        match self {
            EqBand::Low => 0,
            EqBand::Mid => 1,
            EqBand::High => 2,
        }
    }
}

/// Coefficients d'une bande pour un gain en dB (clampé à la plage specs).
/// À appeler côté non temps-réel : la commande transporte le résultat.
pub fn eq_coeffs(band: EqBand, gain_db: f64, sample_rate: f64) -> BiquadCoeffs {
    let gain_db = gain_db.clamp(EQ_MIN_DB, EQ_MAX_DB);
    match band {
        EqBand::Low => biquad::low_shelf(LOW_SHELF_HZ, gain_db, sample_rate),
        EqBand::Mid => biquad::peaking(PEAK_HZ, gain_db, PEAK_Q, sample_rate),
        EqBand::High => biquad::high_shelf(HIGH_SHELF_HZ, gain_db, sample_rate),
    }
}

/// EQ stéréo 3 bandes : coefficients partagés, état par canal.
#[derive(Debug, Clone)]
pub struct StereoEq {
    coeffs: [BiquadCoeffs; 3],
    state: [[BiquadState; 3]; 2],
}

impl Default for StereoEq {
    fn default() -> Self {
        Self {
            coeffs: [BiquadCoeffs::IDENTITY; 3],
            state: [[BiquadState::default(); 3]; 2],
        }
    }
}

impl StereoEq {
    /// Remplace les coefficients d'une bande. L'état est conservé : pas de
    /// click à la modification d'un potard.
    pub fn set_band(&mut self, band: EqBand, coeffs: BiquadCoeffs) {
        self.coeffs[band.index()] = coeffs;
    }

    #[inline]
    pub fn process(&mut self, channel: usize, x: f32) -> f32 {
        let mut y = x;
        for (state, coeffs) in self.state[channel].iter_mut().zip(&self.coeffs) {
            y = state.process(y, coeffs);
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_par_defaut_transparent() {
        let mut eq = StereoEq::default();
        for i in 0..32 {
            let x = (i as f32 * 0.21).cos();
            assert_eq!(eq.process(0, x), x);
            assert_eq!(eq.process(1, x), x);
        }
    }

    #[test]
    fn le_gain_est_clampe_a_la_plage_specs() {
        let flat = eq_coeffs(EqBand::Low, 0.0, 48_000.0);
        let over = eq_coeffs(EqBand::Low, 40.0, 48_000.0);
        let max = eq_coeffs(EqBand::Low, EQ_MAX_DB, 48_000.0);
        assert_eq!(over, max);
        assert_ne!(over, flat);
    }
}
