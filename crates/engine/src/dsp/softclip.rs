//! Limiteur soft-clip du master (specs §3.3) — protection oreilles/enceintes,
//! simple mais obligatoire. `tanh` : transparent en petit signal, saturation
//! progressive vers ±1, jamais d'écrêtage dur.

/// Écrase doucement `x` dans [-1, 1] (mathématiquement ]-1, 1[, mais en f32
/// `tanh` sature à exactement ±1.0 pour |x| grand — la borne reste tenue).
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borne_dans_moins_un_un() {
        for x in [-100.0f32, -2.0, -1.0, 1.0, 2.0, 100.0] {
            let y = soft_clip(x);
            assert!(y.abs() <= 1.0, "x = {x}, y = {y}");
            assert!(y.abs() < x.abs(), "doit atténuer : x = {x}, y = {y}");
            assert_eq!(y.signum(), x.signum());
        }
        assert_eq!(soft_clip(0.0), 0.0);
    }

    #[test]
    fn quasi_transparent_en_petit_signal() {
        for x in [-0.2f32, -0.1, 0.05, 0.2] {
            let y = soft_clip(x);
            assert!((y - x).abs() < 0.015, "x = {x}, y = {y}");
        }
    }

    #[test]
    fn monotone() {
        let mut prev = soft_clip(-4.0);
        let mut x = -4.0f32;
        while x < 4.0 {
            x += 0.05;
            let y = soft_clip(x);
            assert!(y > prev);
            prev = y;
        }
    }
}
