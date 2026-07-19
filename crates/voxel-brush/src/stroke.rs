//! Stroke geometry (`docs/design/brush-editing/04`): stamps, world-space
//! resampling between pointer events, the per-stroke paint mask, the anchor
//! plane, and the symmetry pre-pass.

use std::collections::HashMap;

use glam::DVec3;
use voxel_core::VoxelCoord;

/// Longest gap (in voxels, per unit of radius) a stroke will bridge between
/// two consecutive pointer hits. Within it, stamps interpolate into a
/// continuous groove; beyond it (a drag that jumped to a distant surface) the
/// stroke restarts at the new hit instead of drawing an absurd beam — and the
/// cap also bounds the work one pointer event can queue.
pub(crate) const MAX_BRIDGE_PER_RADIUS: f64 = 8.0;

/// One brush application: a centre voxel and the pen pressure in effect there.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stamp {
    /// The stamp's centre voxel.
    pub center: VoxelCoord,
    /// Pen pressure in `[0, 1]` (1.0 for a mouse), lerped along a resampled
    /// segment so a stroke that lightens mid-drag lightens smoothly.
    pub pressure: f32,
}

/// A plane in voxel space: Flatten's stroke-stable anchor, and (v1.5) the
/// mirror-symmetry plane.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Plane {
    /// A point on the plane (voxel coordinates as f64).
    pub point: DVec3,
    /// Unit normal.
    pub normal: DVec3,
}

/// Per-stroke paint state: for each voxel painted this stroke, its colour at
/// stroke start and the max alpha applied so far. Blending always goes from the
/// *original* colour toward the brush at the running max alpha, so overlapping
/// stamps within one stroke converge to a single application (no banding) while
/// successive strokes still build up. Cleared on `brush_end`.
pub type StrokeMask = HashMap<VoxelCoord, (u32, f32)>;

fn to_v(c: VoxelCoord) -> DVec3 {
    DVec3::new(f64::from(c.x), f64::from(c.y), f64::from(c.z))
}

/// The stamps one pointer event contributes: the hit itself, plus — when the
/// previous stamp is near enough — interpolated stamps filling the world-space
/// segment between them, spaced at half the radius so consecutive balls
/// overlap into a continuous groove. Pressure lerps along the segment.
#[must_use]
pub fn resample(prev: Option<Stamp>, next: Stamp, radius: u32) -> Vec<Stamp> {
    let radius = f64::from(radius.max(1));
    let Some(prev) = prev else {
        return vec![next];
    };
    let (from, to) = (to_v(prev.center), to_v(next.center));
    let dist = from.distance(to);
    if dist > radius * MAX_BRIDGE_PER_RADIUS {
        return vec![next]; // the stroke restarts on the new surface
    }
    let spacing = (radius * 0.5).max(1.0);
    #[allow(clippy::cast_sign_loss)] // dist/spacing >= 0 by construction
    let steps = ((dist / spacing).ceil() as u32).max(1);
    let mut stamps: Vec<Stamp> = Vec::new();
    for i in 1..=steps {
        let frac = f64::from(i) / f64::from(steps);
        let point = from.lerp(to, frac).round();
        #[allow(clippy::cast_sign_loss)] // lerp of non-negative coords
        let center = VoxelCoord::new(point.x as u32, point.y as u32, point.z as u32);
        #[allow(clippy::cast_possible_truncation)] // frac in [0,1]
        let pressure = prev.pressure + (next.pressure - prev.pressure) * frac as f32;
        if stamps.last().map(|s| s.center) != Some(center) && center != prev.center {
            stamps.push(Stamp { center, pressure });
        }
    }
    if stamps.last().map(|s| s.center) != Some(next.center) {
        stamps.push(next);
    } else if let Some(last) = stamps.last_mut() {
        last.pressure = next.pressure; // the endpoint carries the event's pressure
    }
    stamps
}

/// The symmetry pre-pass (`docs/design/brush-editing/03 §symmetry-readiness`):
/// with a mirror plane, each stamp is reflected and appended — every kernel is
/// stamp-local, so mirroring is complete here. `None` (v1) is the identity.
/// Reflections landing at negative coordinates are dropped (off-grid).
#[must_use]
pub fn mirrored(stamps: Vec<Stamp>, plane: Option<Plane>) -> Vec<Stamp> {
    let Some(plane) = plane else {
        return stamps;
    };
    let mut out = Vec::with_capacity(stamps.len() * 2);
    for stamp in stamps {
        out.push(stamp);
        let p = to_v(stamp.center);
        let reflected = p - 2.0 * (p - plane.point).dot(plane.normal) * plane.normal;
        let r = reflected.round();
        if r.min_element() >= 0.0 {
            #[allow(clippy::cast_sign_loss)] // guarded non-negative above
            let center = VoxelCoord::new(r.x as u32, r.y as u32, r.z as u32);
            if center != stamp.center {
                out.push(Stamp {
                    center,
                    pressure: stamp.pressure,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(x: u32, y: u32, z: u32, pressure: f32) -> Stamp {
        Stamp {
            center: VoxelCoord::new(x, y, z),
            pressure,
        }
    }

    #[test]
    fn first_event_is_a_single_stamp() {
        let s = stamp(5, 5, 5, 0.7);
        assert_eq!(resample(None, s, 3), vec![s]);
    }

    /// Gap-free: consecutive stamp centres are at most `spacing` apart, the
    /// endpoint is the event's hit, and the previous centre never repeats.
    #[test]
    fn resample_is_gap_free_and_terminates_at_the_hit() {
        let prev = stamp(2, 10, 10, 1.0);
        let next = stamp(18, 10, 10, 1.0);
        let stamps = resample(Some(prev), next, 4);
        let spacing = 4.0 * 0.5;
        let mut last = super::to_v(prev.center);
        for s in &stamps {
            let here = super::to_v(s.center);
            assert!(last.distance(here) <= spacing + 1.0, "gap at {here:?}");
            assert_ne!(s.center, prev.center);
            last = here;
        }
        assert_eq!(stamps.last().map(|s| s.center), Some(next.center));
    }

    /// Pressure lerps along the segment and hits the endpoint's value exactly.
    #[test]
    fn pressure_lerps_and_hits_the_endpoint() {
        let prev = stamp(0, 10, 10, 0.0);
        let next = stamp(16, 10, 10, 1.0);
        let stamps = resample(Some(prev), next, 4);
        let mut prev_p = 0.0f32;
        for s in &stamps {
            assert!(s.pressure >= prev_p - 1e-6, "pressure must not decrease");
            prev_p = s.pressure;
        }
        assert!((stamps.last().unwrap().pressure - 1.0).abs() < 1e-6);
        assert!(
            stamps.first().unwrap().pressure < 0.5,
            "early stamps carry early pressure"
        );
    }

    #[test]
    fn distant_hits_restart_instead_of_bridging() {
        let prev = stamp(1, 1, 1, 1.0);
        let next = stamp(120, 120, 120, 1.0);
        assert_eq!(resample(Some(prev), next, 1), vec![next]);
    }

    #[test]
    fn mirror_reflects_about_the_plane_and_none_is_identity() {
        let stamps = vec![stamp(10, 5, 5, 0.8)];
        assert_eq!(mirrored(stamps.clone(), None), stamps);
        let plane = Plane {
            point: DVec3::new(16.0, 0.0, 0.0),
            normal: DVec3::X,
        };
        let out = mirrored(stamps, Some(plane));
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].center, VoxelCoord::new(22, 5, 5)); // 16 + (16 − 10)
        assert!((out[1].pressure - 0.8).abs() < 1e-6);
        // A stamp on the plane does not duplicate itself.
        let on_plane = vec![stamp(16, 3, 3, 1.0)];
        assert_eq!(mirrored(on_plane.clone(), Some(plane)), on_plane);
    }
}
