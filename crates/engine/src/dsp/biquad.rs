//! Biquads RBJ (Audio EQ Cookbook, Robert Bristow-Johnson).
//!
//! Les constructeurs de coefficients tournent côté non temps-réel (f64 puis
//! normalisation) ; `BiquadState::process` est la forme directe II transposée,
//! stable numériquement et sans allocation.

/// Coefficients normalisés (a0 = 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    /// Filtre transparent (passe tout).
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Module de la réponse en fréquence à `freq` (Hz). Outil de test et de
    /// visualisation — pas destiné au callback.
    pub fn magnitude_at(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = std::f64::consts::TAU * freq / sample_rate;
        // H(e^jw) = (b0 + b1 e^-jw + b2 e^-2jw) / (1 + a1 e^-jw + a2 e^-2jw)
        let (cos1, sin1) = (w.cos(), w.sin());
        let (cos2, sin2) = ((2.0 * w).cos(), (2.0 * w).sin());
        let num_re = f64::from(self.b0) + f64::from(self.b1) * cos1 + f64::from(self.b2) * cos2;
        let num_im = -(f64::from(self.b1) * sin1 + f64::from(self.b2) * sin2);
        let den_re = 1.0 + f64::from(self.a1) * cos1 + f64::from(self.a2) * cos2;
        let den_im = -(f64::from(self.a1) * sin1 + f64::from(self.a2) * sin2);
        (num_re.hypot(num_im)) / (den_re.hypot(den_im))
    }
}

/// Low-shelf RBJ (pente S = 1).
pub fn low_shelf(freq: f64, gain_db: f64, sample_rate: f64) -> BiquadCoeffs {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = std::f64::consts::TAU * freq / sample_rate;
    let (cos_w0, sin_w0) = (w0.cos(), w0.sin());
    let alpha = sin_w0 / 2.0 * std::f64::consts::SQRT_2;
    let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

    let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
    let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
    let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
    let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
    let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
    let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

    normalize(b0, b1, b2, a0, a1, a2)
}

/// High-shelf RBJ (pente S = 1).
pub fn high_shelf(freq: f64, gain_db: f64, sample_rate: f64) -> BiquadCoeffs {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = std::f64::consts::TAU * freq / sample_rate;
    let (cos_w0, sin_w0) = (w0.cos(), w0.sin());
    let alpha = sin_w0 / 2.0 * std::f64::consts::SQRT_2;
    let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

    let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
    let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
    let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
    let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
    let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
    let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

    normalize(b0, b1, b2, a0, a1, a2)
}

/// Peaking EQ RBJ.
pub fn peaking(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> BiquadCoeffs {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = std::f64::consts::TAU * freq / sample_rate;
    let (cos_w0, sin_w0) = (w0.cos(), w0.sin());
    let alpha = sin_w0 / (2.0 * q);

    let b0 = 1.0 + alpha * a;
    let b1 = -2.0 * cos_w0;
    let b2 = 1.0 - alpha * a;
    let a0 = 1.0 + alpha / a;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha / a;

    normalize(b0, b1, b2, a0, a1, a2)
}

fn normalize(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> BiquadCoeffs {
    BiquadCoeffs {
        b0: (b0 / a0) as f32,
        b1: (b1 / a0) as f32,
        b2: (b2 / a0) as f32,
        a1: (a1 / a0) as f32,
        a2: (a2 / a0) as f32,
    }
}

/// État d'un biquad, forme directe II transposée.
#[derive(Debug, Clone, Copy, Default)]
pub struct BiquadState {
    s1: f32,
    s2: f32,
}

impl BiquadState {
    #[inline]
    pub fn process(&mut self, x: f32, c: &BiquadCoeffs) -> f32 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn low_shelf_booste_les_basses_et_laisse_les_aigus() {
        let c = low_shelf(250.0, 6.0, FS);
        assert!((db(c.magnitude_at(10.0, FS)) - 6.0).abs() < 0.1);
        assert!(db(c.magnitude_at(20_000.0, FS)).abs() < 0.1);
        // À f0, un shelf RBJ passe par la moitié du gain.
        assert!((db(c.magnitude_at(250.0, FS)) - 3.0).abs() < 0.3);
    }

    #[test]
    fn low_shelf_kill_attenue_de_26_db() {
        let c = low_shelf(250.0, -26.0, FS);
        assert!((db(c.magnitude_at(10.0, FS)) + 26.0).abs() < 0.2);
        assert!(db(c.magnitude_at(20_000.0, FS)).abs() < 0.2);
    }

    #[test]
    fn high_shelf_symetrique() {
        let c = high_shelf(2_500.0, -12.0, FS);
        assert!(db(c.magnitude_at(20.0, FS)).abs() < 0.2);
        assert!((db(c.magnitude_at(20_000.0, FS)) + 12.0).abs() < 0.3);
    }

    #[test]
    fn peaking_atteint_son_gain_a_f0() {
        let c = peaking(1_000.0, 6.0, std::f64::consts::FRAC_1_SQRT_2, FS);
        assert!((db(c.magnitude_at(1_000.0, FS)) - 6.0).abs() < 0.1);
        assert!(db(c.magnitude_at(20.0, FS)).abs() < 0.2);
        assert!(db(c.magnitude_at(20_000.0, FS)).abs() < 0.4);
    }

    #[test]
    fn identity_est_transparent_en_filtrage() {
        let mut state = BiquadState::default();
        for i in 0..64 {
            let x = (i as f32 * 0.37).sin();
            let y = state.process(x, &BiquadCoeffs::IDENTITY);
            assert_eq!(x, y);
        }
    }

    #[test]
    fn le_filtrage_temporel_correspond_a_la_reponse_theorique() {
        // Sinus 100 Hz filtré par un low-shelf −26 dB : le RMS de sortie doit
        // suivre la magnitude théorique à 100 Hz.
        let c = low_shelf(250.0, -26.0, FS);
        let mut state = BiquadState::default();
        let n = 48_000;
        let mut sum_in = 0.0f64;
        let mut sum_out = 0.0f64;
        for i in 0..n {
            let x = (std::f64::consts::TAU * 100.0 * i as f64 / FS).sin() as f32;
            let y = state.process(x, &c);
            // Fenêtre de mesure après stabilisation du filtre.
            if i > 2_000 {
                sum_in += f64::from(x * x);
                sum_out += f64::from(y * y);
            }
        }
        let measured = db((sum_out / sum_in).sqrt());
        let expected = db(c.magnitude_at(100.0, FS));
        assert!(
            (measured - expected).abs() < 0.5,
            "mesuré {measured:.2} dB, attendu {expected:.2} dB"
        );
    }
}
