//! v3 CPU reference implementation of WGSL helpers.
//!
//! Every helper in `cascade_common.wgsl` has a CPU mirror here. This module
//! is the ground truth for unit tests — when the WGSL helpers change, the
//! CPU reference must change in lockstep, and the tests in this file
//! exercise both implementations against each other (CPU directly, WGSL via
//! GPU integration tests once they exist).
//!
//! All helpers operate on host-side `glam` types and `u32` indices, with
//! the same semantics as their WGSL counterparts.

use glam::{UVec3, Vec2, Vec3};

use crate::gi::v3::constants::{
    V3_CASCADE_0_DIRS, V3_PROBE_PAYLOAD_BYTES_PER_DIR, V3_PROBE_PAYLOAD_BYTES_PER_PROBE,
    V3_PROBE_PAYLOAD_BYTES_PER_SLOT, V3_PROBES_PER_CHUNK, V3_PROBES_PER_CHUNK_AXIS,
};

// ─── Octahedral encode / decode (full sphere) ─────────────────────────────

/// Encode a unit direction vector to an octahedral (u, v) ∈ [0, 1]² coord.
/// Standard full-sphere octahedral mapping. Inverse of [`oct_decode`].
pub fn oct_encode(dir: Vec3) -> Vec2 {
    let n = dir / (dir.x.abs() + dir.y.abs() + dir.z.abs());
    let octant = if n.y >= 0.0 {
        Vec2::new(n.x, n.z)
    } else {
        Vec2::new(
            (1.0 - n.z.abs()) * sign_not_zero(n.x),
            (1.0 - n.x.abs()) * sign_not_zero(n.z),
        )
    };
    // Map [-1, 1] → [0, 1]
    octant * 0.5 + Vec2::new(0.5, 0.5)
}

/// Decode an octahedral (u, v) ∈ [0, 1]² coord to a unit direction.
/// Inverse of [`oct_encode`].
pub fn oct_decode(uv: Vec2) -> Vec3 {
    let f = uv * 2.0 - Vec2::new(1.0, 1.0);
    let mut n = Vec3::new(f.x, 1.0 - f.x.abs() - f.y.abs(), f.y);
    let t = (-n.y).max(0.0);
    n.x += if n.x >= 0.0 { -t } else { t };
    n.z += if n.z >= 0.0 { -t } else { t };
    n.normalize()
}

fn sign_not_zero(x: f32) -> f32 {
    if x >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Decode the direction stored at integer texel `(dir_x, dir_y)` in an
/// `dirs_per_axis × dirs_per_axis` octahedral grid. Texel centers are at
/// `(dir_x + 0.5, dir_y + 0.5)`.
pub fn oct_decode_texel(dir_x: u32, dir_y: u32, dirs_per_axis: u32) -> Vec3 {
    let uv = Vec2::new(
        (dir_x as f32 + 0.5) / dirs_per_axis as f32,
        (dir_y as f32 + 0.5) / dirs_per_axis as f32,
    );
    oct_decode(uv)
}

// ─── Probe addressing ─────────────────────────────────────────────────────

/// Linearize a probe's local 3D coordinate within a chunk.
///
/// Order: x, y, z (x varies fastest). Matches the WGSL helper.
pub fn probe_index_in_chunk(local_probe: UVec3) -> u32 {
    debug_assert!(local_probe.x < V3_PROBES_PER_CHUNK_AXIS);
    debug_assert!(local_probe.y < V3_PROBES_PER_CHUNK_AXIS);
    debug_assert!(local_probe.z < V3_PROBES_PER_CHUNK_AXIS);
    local_probe.x
        + local_probe.y * V3_PROBES_PER_CHUNK_AXIS
        + local_probe.z * V3_PROBES_PER_CHUNK_AXIS * V3_PROBES_PER_CHUNK_AXIS
}

/// Compute the byte offset of a single direction texel within the
/// probe payload SSBO.
///
/// Layout (slot-major, probe-major, dir-minor):
/// `[slot 0: [probe 0: [dir 0..63], probe 1: [dir 0..63], ...], slot 1: ...]`
pub fn flat_payload_byte_offset(probe_slot: u32, probe_idx: u32, dir_idx: u32) -> usize {
    let slot_off = probe_slot as usize * V3_PROBE_PAYLOAD_BYTES_PER_SLOT as usize;
    let probe_off = probe_idx as usize * V3_PROBE_PAYLOAD_BYTES_PER_PROBE as usize;
    let dir_off = dir_idx as usize * V3_PROBE_PAYLOAD_BYTES_PER_DIR as usize;
    slot_off + probe_off + dir_off
}

/// Same as [`flat_payload_byte_offset`] but in units of `vec4<f16>`
/// (8 bytes), the natural element size of the rgba16f payload.
pub fn flat_payload_texel_index(probe_slot: u32, probe_idx: u32, dir_idx: u32) -> usize {
    let slot_off = probe_slot as usize * V3_PROBES_PER_CHUNK as usize * V3_CASCADE_0_DIRS as usize;
    let probe_off = probe_idx as usize * V3_CASCADE_0_DIRS as usize;
    let dir_off = dir_idx as usize;
    slot_off + probe_off + dir_off
}

// ─── Trilinear weights ────────────────────────────────────────────────────

/// Compute the 8 trilinear weights for a fractional position `frac` in
/// the unit cube `[0, 1]³`. Output order matches the corner index
/// `(corner.x | (corner.y << 1) | (corner.z << 2))` where each component
/// of `corner` is 0 or 1.
pub fn trilinear_weights(frac: Vec3) -> [f32; 8] {
    let fx = frac.x.clamp(0.0, 1.0);
    let fy = frac.y.clamp(0.0, 1.0);
    let fz = frac.z.clamp(0.0, 1.0);
    let (ix, iy, iz) = (1.0 - fx, 1.0 - fy, 1.0 - fz);
    [
        ix * iy * iz, // (0,0,0)
        fx * iy * iz, // (1,0,0)
        ix * fy * iz, // (0,1,0)
        fx * fy * iz, // (1,1,0)
        ix * iy * fz, // (0,0,1)
        fx * iy * fz, // (1,0,1)
        ix * fy * fz, // (0,1,1)
        fx * fy * fz, // (1,1,1)
    ]
}

// ─── Front-to-back over operator (Sannikov Eq. 13) ────────────────────────

/// Front-to-back radiance composition. `near` is the closer interval,
/// `far` is the more distant interval being merged in. Both are
/// `(rgb_radiance, opacity)` pairs.
///
/// Used at shade time once Phase B introduces the cascade hierarchy.
/// Included now so the operator can be unit-tested before it ships.
pub fn over_blend(near: glam::Vec4, far: glam::Vec4) -> glam::Vec4 {
    let near_rgb = near.truncate();
    let far_rgb = far.truncate();
    let near_opacity = near.w;
    let far_opacity = far.w;
    let combined_rgb = near_rgb + (1.0 - near_opacity) * far_rgb;
    let combined_opacity = near_opacity + (1.0 - near_opacity) * far_opacity;
    glam::Vec4::new(
        combined_rgb.x,
        combined_rgb.y,
        combined_rgb.z,
        combined_opacity,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gi::v3::constants::{
        V3_CASCADE_0_DIRS_PER_AXIS, V3_MAX_PROBE_SLOTS, V3_PROBE_PAYLOAD_BUF_BYTES,
    };

    // ─── Octahedral ─────────────────────────────────────────────────────

    #[test]
    fn oct_encode_decode_roundtrip_unit_axes() {
        let cases = [
            Vec3::X,
            -Vec3::X,
            Vec3::Y,
            -Vec3::Y,
            Vec3::Z,
            -Vec3::Z,
            Vec3::new(1.0, 1.0, 1.0).normalize(),
            Vec3::new(-1.0, 1.0, 1.0).normalize(),
            Vec3::new(1.0, -1.0, 1.0).normalize(),
            Vec3::new(1.0, 1.0, -1.0).normalize(),
        ];
        for d in cases {
            let uv = oct_encode(d);
            let decoded = oct_decode(uv);
            let err = (decoded - d).length();
            assert!(
                err < 1e-4,
                "round-trip failure: dir={d:?} uv={uv:?} decoded={decoded:?} err={err}",
            );
        }
    }

    #[test]
    fn oct_encode_decode_roundtrip_random() {
        // Pseudo-random directions via fixed Halton-like sequence (no RNG dep).
        let mut max_err = 0.0_f32;
        for i in 0..256 {
            let theta = (i as f32 * 0.7853981633) % (std::f32::consts::PI * 2.0);
            let phi = ((i as f32 * 0.3826834323) % 1.0) * std::f32::consts::PI;
            let dir = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin())
                .normalize();
            let decoded = oct_decode(oct_encode(dir));
            let err = (decoded - dir).length();
            max_err = max_err.max(err);
        }
        assert!(max_err < 1e-4, "max round-trip error {max_err} > 1e-4");
    }

    #[test]
    fn oct_decode_texel_centers_are_unit_vectors() {
        for dy in 0..V3_CASCADE_0_DIRS_PER_AXIS {
            for dx in 0..V3_CASCADE_0_DIRS_PER_AXIS {
                let dir = oct_decode_texel(dx, dy, V3_CASCADE_0_DIRS_PER_AXIS);
                let len = dir.length();
                assert!(
                    (len - 1.0).abs() < 1e-4,
                    "non-unit direction at texel ({dx},{dy}): len={len}",
                );
            }
        }
    }

    #[test]
    fn oct_full_sphere_coverage() {
        // The 64 cascade-0 texels should span the full sphere — sum of
        // direction vectors should be close to zero (cancelling pairs).
        let mut sum = Vec3::ZERO;
        for dy in 0..V3_CASCADE_0_DIRS_PER_AXIS {
            for dx in 0..V3_CASCADE_0_DIRS_PER_AXIS {
                sum += oct_decode_texel(dx, dy, V3_CASCADE_0_DIRS_PER_AXIS);
            }
        }
        let avg_len = sum.length() / V3_CASCADE_0_DIRS as f32;
        assert!(
            avg_len < 0.05,
            "directions don't span full sphere: avg residual {avg_len}",
        );
    }

    // ─── Probe addressing ───────────────────────────────────────────────

    #[test]
    fn probe_index_in_chunk_is_x_major() {
        assert_eq!(probe_index_in_chunk(UVec3::new(0, 0, 0)), 0);
        assert_eq!(probe_index_in_chunk(UVec3::new(1, 0, 0)), 1);
        assert_eq!(
            probe_index_in_chunk(UVec3::new(0, 1, 0)),
            V3_PROBES_PER_CHUNK_AXIS,
        );
        assert_eq!(
            probe_index_in_chunk(UVec3::new(0, 0, 1)),
            V3_PROBES_PER_CHUNK_AXIS * V3_PROBES_PER_CHUNK_AXIS,
        );
        // Last probe in chunk
        let last = V3_PROBES_PER_CHUNK_AXIS - 1;
        assert_eq!(
            probe_index_in_chunk(UVec3::new(last, last, last)),
            V3_PROBES_PER_CHUNK - 1,
        );
    }

    #[test]
    fn probe_index_in_chunk_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for z in 0..V3_PROBES_PER_CHUNK_AXIS {
            for y in 0..V3_PROBES_PER_CHUNK_AXIS {
                for x in 0..V3_PROBES_PER_CHUNK_AXIS {
                    let idx = probe_index_in_chunk(UVec3::new(x, y, z));
                    assert!(seen.insert(idx), "duplicate probe index {idx}");
                    assert!(idx < V3_PROBES_PER_CHUNK);
                }
            }
        }
        assert_eq!(seen.len(), V3_PROBES_PER_CHUNK as usize);
    }

    #[test]
    fn flat_payload_offset_no_overlap() {
        // Test that consecutive (slot, probe, dir) triples have distinct offsets
        // and don't overflow the buffer.
        let cases = [
            (0, 0, 0),
            (0, 0, 1),
            (0, 0, V3_CASCADE_0_DIRS - 1),
            (0, 1, 0),
            (0, V3_PROBES_PER_CHUNK - 1, V3_CASCADE_0_DIRS - 1),
            (1, 0, 0),
            (V3_MAX_PROBE_SLOTS - 1, V3_PROBES_PER_CHUNK - 1, V3_CASCADE_0_DIRS - 1),
        ];
        let mut prev = 0usize;
        for (slot, probe, dir) in cases {
            let off = flat_payload_byte_offset(slot, probe, dir);
            assert!(off < V3_PROBE_PAYLOAD_BUF_BYTES as usize);
            if (slot, probe, dir) != (0, 0, 0) {
                assert!(off > prev, "offsets not strictly increasing");
            }
            prev = off;
        }
    }

    #[test]
    fn flat_payload_byte_and_texel_index_agree() {
        for &(slot, probe, dir) in &[
            (0u32, 0u32, 0u32),
            (3, 17, 42),
            (V3_MAX_PROBE_SLOTS - 1, V3_PROBES_PER_CHUNK - 1, V3_CASCADE_0_DIRS - 1),
        ] {
            let bytes = flat_payload_byte_offset(slot, probe, dir);
            let texels = flat_payload_texel_index(slot, probe, dir);
            assert_eq!(
                bytes,
                texels * V3_PROBE_PAYLOAD_BYTES_PER_DIR as usize,
                "byte and texel offsets disagree at ({slot},{probe},{dir})",
            );
        }
    }

    // ─── Trilinear weights ──────────────────────────────────────────────

    #[test]
    fn trilinear_weights_partition_unity() {
        let cases = [
            Vec3::ZERO,
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.1, 0.7, 0.3),
            Vec3::new(0.99, 0.01, 0.5),
        ];
        for f in cases {
            let w = trilinear_weights(f);
            let sum: f32 = w.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "weights don't sum to 1 at frac={f:?}: sum={sum} weights={w:?}",
            );
            for &wi in &w {
                assert!((0.0..=1.0).contains(&wi), "weight out of range: {wi}");
            }
        }
    }

    #[test]
    fn trilinear_weights_at_corner_are_one_hot() {
        // At frac=(0,0,0) only the (0,0,0) corner has weight 1.
        let w = trilinear_weights(Vec3::ZERO);
        assert!((w[0] - 1.0).abs() < 1e-6);
        for i in 1..8 {
            assert!(w[i].abs() < 1e-6);
        }
        // At frac=(1,1,1) only the (1,1,1) corner has weight 1.
        let w = trilinear_weights(Vec3::ONE);
        assert!((w[7] - 1.0).abs() < 1e-6);
        for i in 0..7 {
            assert!(w[i].abs() < 1e-6);
        }
    }

    // ─── Over operator ──────────────────────────────────────────────────

    #[test]
    fn over_blend_opaque_near_blocks_far() {
        let near = glam::Vec4::new(1.0, 0.0, 0.0, 1.0); // opaque red
        let far = glam::Vec4::new(0.0, 1.0, 0.0, 1.0); // green (occluded)
        let blended = over_blend(near, far);
        assert!((blended.x - 1.0).abs() < 1e-5);
        assert!(blended.y.abs() < 1e-5);
        assert!((blended.w - 1.0).abs() < 1e-5);
    }

    #[test]
    fn over_blend_transparent_near_passes_far() {
        let near = glam::Vec4::new(0.5, 0.0, 0.0, 0.0); // transparent
        let far = glam::Vec4::new(0.0, 1.0, 0.0, 1.0);
        let blended = over_blend(near, far);
        assert!((blended.x - 0.5).abs() < 1e-5);
        assert!((blended.y - 1.0).abs() < 1e-5);
        assert!((blended.w - 1.0).abs() < 1e-5);
    }

    #[test]
    fn over_blend_associates_correctly() {
        // (a over b) over c == a over (b over c)
        let a = glam::Vec4::new(0.3, 0.0, 0.0, 0.4);
        let b = glam::Vec4::new(0.0, 0.5, 0.0, 0.3);
        let c = glam::Vec4::new(0.0, 0.0, 0.7, 0.5);
        let lhs = over_blend(over_blend(a, b), c);
        let rhs = over_blend(a, over_blend(b, c));
        let diff = (lhs - rhs).length();
        assert!(diff < 1e-5, "over operator not associative: diff={diff}");
    }
}
