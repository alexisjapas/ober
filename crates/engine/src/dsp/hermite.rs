//! Interpolation Hermite cubique 4 points (specs §3.3) — le varispeed lit la
//! piste à position fractionnaire. Formulation de Laurent de Soras (usuelle
//! en audio), 3ᵉ ordre, continue en valeur et en pente.

/// Interpole entre `x0` (t = 0) et `x1` (t = 1), avec les voisins `xm1`/`x2`.
#[inline]
pub fn hermite4(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c = (x1 - xm1) * 0.5;
    let v = x0 - x1;
    let w = c + v;
    let a = w + v + (x2 - x0) * 0.5;
    let b_neg = w + a;
    ((a * t - b_neg) * t + c) * t + x0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passe_par_les_points_de_controle() {
        let (xm1, x0, x1, x2) = (0.3, -0.5, 0.8, 0.1);
        assert!((hermite4(xm1, x0, x1, x2, 0.0) - x0).abs() < 1e-7);
        assert!((hermite4(xm1, x0, x1, x2, 1.0) - x1).abs() < 1e-6);
    }

    #[test]
    fn exact_sur_une_rampe_lineaire() {
        // Un polynôme cubique interpolant reproduit exactement une droite.
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let y = hermite4(-1.0, 0.0, 1.0, 2.0, t);
            assert!((y - t).abs() < 1e-6, "t = {t}, y = {y}");
        }
    }

    #[test]
    fn erreur_faible_sur_un_sinus_sous_echantillonne() {
        // Sinus à ~1/20 de la fréquence d'échantillonnage, lecture à mi-chemin
        // entre deux samples : l'erreur doit rester très faible.
        let f = |i: f32| (0.3 * i).sin();
        let mut max_err = 0.0f32;
        for i in 2..40 {
            let i = i as f32;
            let y = hermite4(f(i - 1.0), f(i), f(i + 1.0), f(i + 2.0), 0.5);
            max_err = max_err.max((y - f(i + 0.5)).abs());
        }
        assert!(max_err < 2e-3, "erreur max = {max_err}");
    }
}
