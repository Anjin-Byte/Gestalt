//! The falloff curves (`docs/design/brush-editing/03 §falloff-curves`).

use crate::params::Falloff;

/// The falloff weight at normalized radius `t` — `1` at the centre, `0` at the
/// rim, monotone non-increasing. See [`Falloff`].
#[must_use]
pub fn weight(f: Falloff, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match f {
        Falloff::Smooth => 1.0 - (3.0 * t * t - 2.0 * t * t * t),
        Falloff::Linear => 1.0 - t,
        Falloff::Sphere => (1.0 - t * t).max(0.0).sqrt(),
        Falloff::Sharp => (1.0 - t) * (1.0 - t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_curves_are_bounded_and_monotone() {
        for f in [
            Falloff::Smooth,
            Falloff::Linear,
            Falloff::Sphere,
            Falloff::Sharp,
        ] {
            assert!((weight(f, 0.0) - 1.0).abs() < 1e-6, "{f:?} w(0)");
            assert!(weight(f, 1.0).abs() < 1e-6, "{f:?} w(1)");
            let mut prev = weight(f, 0.0);
            for i in 1..=20u8 {
                let w = weight(f, f32::from(i) / 20.0);
                assert!(
                    w <= prev + 1e-6 && (0.0..=1.0).contains(&w),
                    "{f:?} monotone at {i}"
                );
                prev = w;
            }
        }
    }

    #[test]
    fn out_of_range_t_clamps() {
        for f in [
            Falloff::Smooth,
            Falloff::Linear,
            Falloff::Sphere,
            Falloff::Sharp,
        ] {
            assert!((weight(f, -1.0) - 1.0).abs() < 1e-6);
            assert!(weight(f, 2.0).abs() < 1e-6);
        }
    }
}
