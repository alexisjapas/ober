//! Modèle de jog scratch/bend à inertie (specs §3.5) — le point le plus
//! délicat du projet. État par deck, avancé frame par frame par le graphe,
//! sans allocation.
//!
//! - **Scratch** (surface touchée) : les ticks relatifs sont convertis en
//!   vélocité cible par une fenêtre glissante (~15 ms), puis la vitesse de
//!   lecture est asservie par un passe-bas (τ ≈ 5 ms) pour éviter le son
//!   « escalier ». À la prise en main, l'asservissement démarre de la
//!   vitesse courante (freinage naturel du plateau).
//! - **Relâchement** : rampe linéaire de retour à la vitesse nominale
//!   (50–200 ms, configurable) — nominale = 0 si le deck est à l'arrêt.
//! - **Bend** (bord, sans touch) : offset de vitesse proportionnel à la
//!   vélocité de rotation, retour progressif (passe-bas vers 0 quand la
//!   rotation cesse). Sans effet sur un deck à l'arrêt.
//!
//! Tous les paramètres viennent du mapping RON via
//! [`EngineCommand::SetJogParams`](crate::command::EngineCommand).

use crate::PREFERRED_SAMPLE_RATE;
use crate::command::JogParams;

/// Per-sample coefficients, precomputed **outside** the audio callback from
/// [`JogParams`] and the sample rate of the opened stream (Rule 5: commands
/// carry ready-to-use values, like EQ coefficients).
#[derive(Debug, Clone, Copy)]
pub struct JogRuntime {
    touch_scratch: bool,
    window_samples: u32,
    /// ticks de la fenêtre → vitesse (multiples de la vitesse nominale du
    /// plateau virtuel).
    ticks_to_speed: f64,
    alpha_scratch: f64,
    alpha_bend: f64,
    release_samples: f64,
    bend_sensitivity: f64,
}

impl JogRuntime {
    /// Derives the per-sample coefficients for the given stream rate.
    pub fn new(p: JogParams, sample_rate: u32) -> Self {
        let fs = f64::from(sample_rate);
        let window_samples = (p.velocity_window * fs).max(1.0);
        Self {
            touch_scratch: p.touch_scratch,
            window_samples: window_samples as u32,
            ticks_to_speed: 1.0
                / (p.ticks_per_rev.max(1.0) * (window_samples / fs) * p.platter_rev_per_s),
            alpha_scratch: one_pole_alpha(p.scratch_smoothing, fs),
            alpha_bend: one_pole_alpha(p.bend_return, fs),
            release_samples: (p.release_ramp * fs).max(1.0),
            bend_sensitivity: p.bend_sensitivity,
        }
    }
}

impl Default for JogRuntime {
    fn default() -> Self {
        Self::new(JogParams::default(), PREFERRED_SAMPLE_RATE)
    }
}

fn one_pole_alpha(tau_seconds: f64, fs: f64) -> f64 {
    if tau_seconds <= 0.0 {
        1.0
    } else {
        1.0 - (-1.0 / (tau_seconds * fs)).exp()
    }
}

/// État du jog d'un deck.
#[derive(Debug, Clone, Copy, Default)]
pub struct JogState {
    touched: bool,
    window_ticks: f64,
    window_pos: u32,
    /// Vélocité du jog estimée sur la dernière fenêtre (multiples de la
    /// vitesse nominale du plateau).
    velocity: f64,
    /// Vitesse de lecture asservie en mode scratch.
    scratch_speed: f64,
    /// Offset de vitesse lissé en mode bend.
    bend_offset: f64,
    release_remaining: f64,
    release_from: f64,
}

impl JogState {
    /// Touch capacitif. `current_speed` : vitesse de lecture effective au
    /// moment de la prise en main (continuité du freinage).
    pub fn set_touched(&mut self, touched: bool, current_speed: f64, rt: &JogRuntime) {
        if !rt.touch_scratch {
            return;
        }
        if touched && !self.touched {
            self.scratch_speed = current_speed;
            self.velocity = 0.0;
            self.window_ticks = 0.0;
            self.window_pos = 0;
            self.release_remaining = 0.0;
        }
        if !touched && self.touched {
            self.release_from = self.scratch_speed;
            self.release_remaining = rt.release_samples;
        }
        self.touched = touched;
    }

    pub fn add_ticks(&mut self, ticks: i32) {
        self.window_ticks += f64::from(ticks);
    }

    /// Vrai si le jog impose de traiter le deck même à l'arrêt (scratch en
    /// cours ou rampe de relâchement).
    pub fn engaged(&self, rt: &JogRuntime) -> bool {
        (self.touched && rt.touch_scratch) || self.release_remaining > 0.0
    }

    /// Vitesse de lecture effective pour la frame courante. Fait avancer
    /// l'horloge de la fenêtre d'estimation — à appeler exactement une fois
    /// par frame traitée.
    #[inline]
    pub fn effective_speed(&mut self, nominal: f64, rt: &JogRuntime) -> f64 {
        self.window_pos += 1;
        if self.window_pos >= rt.window_samples {
            self.velocity = self.window_ticks * rt.ticks_to_speed;
            self.window_ticks = 0.0;
            self.window_pos = 0;
        }

        if self.touched && rt.touch_scratch {
            self.scratch_speed += rt.alpha_scratch * (self.velocity - self.scratch_speed);
            self.scratch_speed
        } else if self.release_remaining > 0.0 {
            self.release_remaining -= 1.0;
            let t = 1.0 - self.release_remaining / rt.release_samples;
            self.release_from + (nominal - self.release_from) * t
        } else if nominal != 0.0 {
            let target = self.velocity * rt.bend_sensitivity;
            self.bend_offset += rt.alpha_bend * (target - self.bend_offset);
            nominal + self.bend_offset
        } else {
            self.bend_offset = 0.0;
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> JogRuntime {
        JogRuntime::default()
    }

    #[test]
    fn scratch_converge_vers_la_velocite_du_jog() {
        let rt = rt();
        let mut jog = JogState::default();
        jog.set_touched(true, 0.0, &rt);

        // Plateau tourné à ~2× la vitesse nominale : 720 ticks/tour ×
        // 0,5556 tr/s × 2 = 800 ticks/s ≈ 12 ticks par fenêtre de 15 ms.
        let mut speed = 0.0;
        for _ in 0..20 {
            jog.add_ticks(12);
            for _ in 0..rt.window_samples {
                speed = jog.effective_speed(0.0, &rt);
            }
        }
        assert!((speed - 2.0).abs() < 0.1, "vitesse = {speed}");
    }

    #[test]
    fn relachement_rampe_vers_la_nominale() {
        let rt = rt();
        let mut jog = JogState::default();
        jog.set_touched(true, 1.0, &rt);
        // Main posée immobile : la vitesse est freinée vers 0.
        for _ in 0..4_800 {
            jog.effective_speed(1.0, &rt);
        }
        let held = jog.effective_speed(1.0, &rt);
        assert!(held.abs() < 0.02, "plateau tenu ≈ arrêté : {held}");

        // Relâchement : retour progressif vers la nominale (1.0).
        jog.set_touched(false, held, &rt);
        assert!(jog.engaged(&rt));
        let mut mid = 0.0;
        let release_samples = (0.1 * 48_000.0) as usize;
        for i in 0..release_samples {
            let v = jog.effective_speed(1.0, &rt);
            if i == release_samples / 2 {
                mid = v;
            }
        }
        assert!(
            mid > 0.3 && mid < 0.7,
            "à mi-rampe la vitesse doit être intermédiaire : {mid}"
        );
        let end = jog.effective_speed(1.0, &rt);
        assert!((end - 1.0).abs() < 0.05, "fin de rampe ≈ nominale : {end}");
        assert!(!jog.engaged(&rt));
    }

    #[test]
    fn bend_offset_proportionnel_puis_retour() {
        let rt = rt();
        let mut jog = JogState::default();

        // Rotation du bord à vitesse nominale (velocity ≈ 1) sans touch.
        let mut speed = 1.0;
        for _ in 0..40 {
            jog.add_ticks(6); // ≈ 400 ticks/s = vitesse nominale
            for _ in 0..rt.window_samples {
                speed = jog.effective_speed(1.0, &rt);
            }
        }
        let expected = 1.0 + 0.3; // bend_sensitivity = 0.3
        assert!((speed - expected).abs() < 0.05, "vitesse bend = {speed}");

        // Rotation stoppée : retour progressif à la nominale.
        for _ in 0..48_000 {
            speed = jog.effective_speed(1.0, &rt);
        }
        assert!((speed - 1.0).abs() < 0.01, "retour à la nominale : {speed}");
    }

    #[test]
    fn bend_sans_lecture_est_inerte() {
        let rt = rt();
        let mut jog = JogState::default();
        jog.add_ticks(100);
        for _ in 0..2_000 {
            assert_eq!(jog.effective_speed(0.0, &rt), 0.0);
        }
        assert!(!jog.engaged(&rt));
    }
}
