//! Surface-normal estimation (`docs/design/brush-editing/03 §normal-estimation`):
//! the occupancy gradient over the brush ball — the stable "perceived surface"
//! direction, deliberately **not** the DDA entry face (which flips per pixel on
//! a bumpy voxel surface).

use glam::DVec3;
use voxel_core::VoxelCoord;

use crate::kernels::Field;

/// The estimated outward surface normal at `c`: the direction of decreasing
/// occupancy density over the radius-`r` ball — `-normalize(Σ dir(d))` over
/// occupied offsets `d`. `None` when the neighborhood is degenerate (isolated
/// voxel, symmetric fill); callers fall back to facing the viewer.
#[must_use]
pub fn estimate_normal<F: Field>(field: &F, c: VoxelCoord, r: u32) -> Option<DVec3> {
    let r = i64::from(r.max(1));
    let r2 = r * r;
    let mut sum = DVec3::ZERO;
    for dz in -r..=r {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy + dz * dz > r2 || (dx, dy, dz) == (0, 0, 0) {
                    continue;
                }
                let p = (
                    i64::from(c.x) + dx,
                    i64::from(c.y) + dy,
                    i64::from(c.z) + dz,
                );
                if field.occupied(p.0, p.1, p.2) {
                    #[allow(clippy::cast_precision_loss)] // |d| ≤ 12
                    let d = DVec3::new(dx as f64, dy as f64, dz as f64);
                    sum += d.normalize();
                }
            }
        }
    }
    // A near-zero sum means no usable asymmetry: don't manufacture a direction.
    (sum.length() > 0.5).then(|| -sum.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A half-space field `n·(p − p0) ≤ 0` (occupied below the plane).
    struct HalfSpace {
        n: DVec3,
        p0: DVec3,
    }
    impl Field for HalfSpace {
        fn occupied(&self, x: i64, y: i64, z: i64) -> bool {
            #[allow(clippy::cast_precision_loss)]
            let p = DVec3::new(x as f64, y as f64, z as f64);
            self.n.dot(p - self.p0) <= 0.0
        }
        fn color(&self, _x: i64, _y: i64, _z: i64) -> u32 {
            0
        }
    }

    /// Random half-space fields ⇒ the estimate lands within 10° of the plane
    /// normal (the 09 oracle). Directions from a fixed low-discrepancy sweep —
    /// deterministic, no RNG.
    #[test]
    fn half_space_normals_land_within_ten_degrees() {
        let center = VoxelCoord::new(32, 32, 32);
        let p0 = DVec3::new(32.0, 32.0, 32.0);
        for i in 0..40u32 {
            let azimuth = f64::from(i) * 0.618_034 * std::f64::consts::TAU;
            let elev = f64::from(i).mul_add(0.049, -0.98).clamp(-0.98, 0.98);
            let ring = (1.0 - elev * elev).sqrt();
            let n = DVec3::new(ring * azimuth.cos(), ring * azimuth.sin(), elev).normalize();
            let field = HalfSpace { n, p0 };
            let est = estimate_normal(&field, center, 6).expect("half-space is not degenerate");
            let angle = est.dot(n).clamp(-1.0, 1.0).acos().to_degrees();
            assert!(angle <= 10.0, "dir {i}: {angle:.2}° off (n = {n:?})");
        }
    }

    struct Solid;
    impl Field for Solid {
        fn occupied(&self, _x: i64, _y: i64, _z: i64) -> bool {
            true
        }
        fn color(&self, _x: i64, _y: i64, _z: i64) -> u32 {
            0
        }
    }

    struct Lone;
    impl Field for Lone {
        fn occupied(&self, x: i64, y: i64, z: i64) -> bool {
            (x, y, z) == (10, 10, 10)
        }
        fn color(&self, _x: i64, _y: i64, _z: i64) -> u32 {
            0
        }
    }

    #[test]
    fn symmetric_and_isolated_fields_are_degenerate() {
        assert_eq!(
            estimate_normal(&Solid, VoxelCoord::new(10, 10, 10), 4),
            None
        );
        assert_eq!(estimate_normal(&Lone, VoxelCoord::new(10, 10, 10), 4), None);
    }
}
