//! The seven brush kernels (`docs/design/brush-editing/03 §the-seven-kernels`):
//! a read-only [`Field`] and one [`Stamp`] in, a deterministic op list out.
//!
//! Every kernel computes against the **immutable pre-stamp field** — a `Set`
//! emitted for one voxel never enables a neighbour within the same stamp, so
//! op lists are order-free and reproducible; layering comes from successive
//! stamps. The one kernel that needs intermediate state (Smooth's two passes)
//! runs them through a private overlay and emits only the final diff.

use std::collections::HashMap;

use voxel_core::{VoxelCoord, brush_voxels};

use crate::falloff::weight;
use crate::params::{BrushParams, BrushTool, MAX_BRUSH_RADIUS};
use crate::stroke::{Plane, Stamp, StrokeMask};

/// The read-only view a kernel samples. Implemented by a thin adapter over
/// `SparseTree` in `voxel-web`, and by plain grids in tests — which is what
/// makes every kernel testable against brute-force references without a tree
/// in sight. Coordinates are signed so off-grid probes are expressible; off-
/// grid is unoccupied.
pub trait Field {
    /// Whether the voxel at `(x, y, z)` is occupied (off-grid is not).
    fn occupied(&self, x: i64, y: i64, z: i64) -> bool;
    /// sRGB RGBA8 (R low) at an occupied voxel; unspecified elsewhere.
    fn color(&self, x: i64, y: i64, z: i64) -> u32;
}

/// One voxel mutation, in tree terms. The controller applies these through the
/// tree's edit API (`Set` → `set_voxel_colored`, `Clear` → `set_voxel(false)`,
/// `Recolor` → `set_color`) with the undo journal capturing at that seam.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoxelOp {
    /// Ensure occupied, carrying this colour (inert on a colourless tree).
    Set {
        /// The voxel to set.
        c: VoxelCoord,
        /// Its colour (sRGB RGBA8, R low).
        rgba: u32,
    },
    /// Ensure empty.
    Clear {
        /// The voxel to clear.
        c: VoxelCoord,
    },
    /// Colour-only write on an occupied voxel (dropped on a colourless tree).
    /// The rgba is the **resolved** post-blend colour: Paint's max-alpha mask
    /// and linear-light blend run kernel-side, so application stays trivial
    /// and the rebuild-parity oracle stays sharp.
    Recolor {
        /// The voxel to recolour.
        c: VoxelCoord,
        /// The resolved colour (sRGB RGBA8, R low).
        rgba: u32,
    },
}

impl VoxelOp {
    /// The voxel this op touches.
    #[must_use]
    pub fn coord(&self) -> VoxelCoord {
        match *self {
            VoxelOp::Set { c, .. } | VoxelOp::Clear { c } | VoxelOp::Recolor { c, .. } => c,
        }
    }
}

/// The ops one stamp of the current tool produces. `anchor` is Flatten's
/// stroke-stable plane (captured by the controller at stroke start; without it
/// Flatten is a no-op). `mask` is the per-stroke paint mask (Paint only).
#[must_use]
pub fn stamp_ops<F: Field>(
    field: &F,
    params: &BrushParams,
    stamp: &Stamp,
    anchor: Option<Plane>,
    mask: &mut StrokeMask,
) -> Vec<VoxelOp> {
    let radius = effective_radius(params.radius.min(MAX_BRUSH_RADIUS), stamp.pressure);
    let s = (params.strength * stamp.pressure).clamp(0.0, 1.0);
    let center = stamp.center;
    match params.tool {
        BrushTool::Draw => ball(center, radius)
            .filter(|&c| !occ(field, c))
            .map(|c| VoxelOp::Set {
                c,
                rgba: params.color,
            })
            .collect(),
        BrushTool::Erase => ball(center, radius)
            .filter(|&c| occ(field, c))
            .map(|c| VoxelOp::Clear { c })
            .collect(),
        BrushTool::Paint => paint_ops(field, params, center, radius, s, mask),
        BrushTool::Clay => ball(center, radius)
            .filter(|&c| {
                !occ(field, c)
                    && has_occupied_6_neighbor(field, c)
                    && weight(params.falloff, t_of(c, center, radius)) * s >= 0.5
            })
            .map(|c| VoxelOp::Set {
                c,
                rgba: neighbor_avg_color(field, c),
            })
            .collect(),
        // Deflate (Alt): the mirror image — exposed surface-shell voxels thin
        // uniformly within the same strength-scaled reach.
        BrushTool::Inflate if params.invert => ball(center, radius)
            .filter(|&c| {
                occ(field, c) && has_empty_6_neighbor(field, c) && t_of(c, center, radius) <= s
            })
            .map(|c| VoxelOp::Clear { c })
            .collect(),
        BrushTool::Inflate => ball(center, radius)
            .filter(|&c| {
                // Uniform within the strength-scaled reach — Clay without the
                // centre bias: fattens the whole covered surface evenly.
                !occ(field, c) && has_occupied_6_neighbor(field, c) && t_of(c, center, radius) <= s
            })
            .map(|c| VoxelOp::Set {
                c,
                rgba: neighbor_avg_color(field, c),
            })
            .collect(),
        BrushTool::Flatten => flatten_ops(field, center, radius, s, anchor),
        BrushTool::Smooth => smooth_ops(field, params, center, radius, s),
    }
}

/// The stamp's ball, as an iterator (pure geometry from voxel-core; negative
/// coordinates already omitted).
fn ball(center: VoxelCoord, radius: u32) -> impl Iterator<Item = VoxelCoord> {
    brush_voxels(center, radius).into_iter()
}

fn occ<F: Field>(field: &F, c: VoxelCoord) -> bool {
    field.occupied(i64::from(c.x), i64::from(c.y), i64::from(c.z))
}

/// Normalized distance from the stamp centre: `t = d / r ∈ [0, 1]`.
fn t_of(c: VoxelCoord, center: VoxelCoord, radius: u32) -> f32 {
    let dx = f64::from(c.x) - f64::from(center.x);
    let dy = f64::from(c.y) - f64::from(center.y);
    let dz = f64::from(c.z) - f64::from(center.z);
    #[allow(clippy::cast_possible_truncation)] // d/r ≤ ~1, exact in f32 terms
    let t = ((dx * dx + dy * dy + dz * dz).sqrt() / f64::from(radius.max(1))) as f32;
    t.min(1.0)
}

const NEIGHBORS_6: [(i64, i64, i64); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

fn has_occupied_6_neighbor<F: Field>(field: &F, c: VoxelCoord) -> bool {
    NEIGHBORS_6.iter().any(|&(dx, dy, dz)| {
        field.occupied(
            i64::from(c.x) + dx,
            i64::from(c.y) + dy,
            i64::from(c.z) + dz,
        )
    })
}

/// Whether `c` has an exposed face (an empty 6-neighbour) — the deflate arm's
/// surface-shell membership test.
fn has_empty_6_neighbor<F: Field>(field: &F, c: VoxelCoord) -> bool {
    NEIGHBORS_6.iter().any(|&(dx, dy, dz)| {
        !field.occupied(
            i64::from(c.x) + dx,
            i64::from(c.y) + dy,
            i64::from(c.z) + dz,
        )
    })
}

/// The Stage-D pressure response curve: pen pressure scales the effective
/// stamp radius between half and full size (a mouse reports 1.0 → identity),
/// so light pen touches make smaller marks as well as weaker ones.
fn effective_radius(radius: u32, pressure: f32) -> u32 {
    let scaled = (radius as f32) * 0.5f32.mul_add(pressure.clamp(0.0, 1.0), 0.5);
    (scaled.round() as u32).max(1)
}

/// The per-channel arithmetic mean of the occupied 6-neighbours' colours
/// (sRGB bytes, alpha forced opaque) — clay carries the surface's paint with
/// it instead of punching default-colour holes. Callers guarantee at least one
/// occupied neighbour.
fn neighbor_avg_color<F: Field>(field: &F, c: VoxelCoord) -> u32 {
    let mut sum = [0u32; 3];
    let mut count = 0u32;
    for &(dx, dy, dz) in &NEIGHBORS_6 {
        let (nx, ny, nz) = (
            i64::from(c.x) + dx,
            i64::from(c.y) + dy,
            i64::from(c.z) + dz,
        );
        if field.occupied(nx, ny, nz) {
            let bytes = field.color(nx, ny, nz).to_le_bytes();
            for (acc, byte) in sum.iter_mut().zip(bytes) {
                *acc += u32::from(byte);
            }
            count += 1;
        }
    }
    debug_assert!(count > 0, "caller guarantees an occupied neighbour");
    let count = count.max(1);
    #[allow(clippy::cast_possible_truncation)] // mean of bytes fits a byte
    u32::from_le_bytes([
        (sum[0] / count) as u8,
        (sum[1] / count) as u8,
        (sum[2] / count) as u8,
        255,
    ])
}

/// Paint: occupied voxels only; `alpha = w · s` max-combined through the
/// per-stroke mask, blended in linear light from the stroke-start colour.
fn paint_ops<F: Field>(
    field: &F,
    params: &BrushParams,
    center: VoxelCoord,
    radius: u32,
    s: f32,
    mask: &mut StrokeMask,
) -> Vec<VoxelOp> {
    let mut ops = Vec::new();
    for c in ball(center, radius) {
        if !occ(field, c) {
            continue;
        }
        let alpha = (weight(params.falloff, t_of(c, center, radius)) * s).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            continue;
        }
        let (orig, applied) = *mask.entry(c).or_insert_with(|| {
            (
                field.color(i64::from(c.x), i64::from(c.y), i64::from(c.z)),
                0.0,
            )
        });
        if alpha <= applied {
            continue;
        }
        mask.insert(c, (orig, alpha));
        ops.push(VoxelOp::Recolor {
            c,
            rgba: blend_srgb(orig, params.color, alpha),
        });
    }
    ops
}

/// Flatten: slab clamp against the stroke-stable anchor plane. Occupied voxels
/// more than half a voxel above the plane cut; empty voxels within the
/// strength-scaled slab just below it, with occupied support, fill.
fn flatten_ops<F: Field>(
    field: &F,
    center: VoxelCoord,
    radius: u32,
    s: f32,
    anchor: Option<Plane>,
) -> Vec<VoxelOp> {
    let Some(plane) = anchor else {
        return Vec::new(); // no anchor captured (a stroke that started on a miss)
    };
    let floor = -f64::from(s) * f64::from(radius.max(1)) / 4.0;
    let mut ops = Vec::new();
    for c in ball(center, radius) {
        let p = glam::DVec3::new(f64::from(c.x), f64::from(c.y), f64::from(c.z));
        let signed = (p - plane.point).dot(plane.normal);
        if occ(field, c) {
            if signed > 0.5 {
                ops.push(VoxelOp::Clear { c });
            }
        } else if signed <= 0.0 && signed >= floor && has_occupied_6_neighbor(field, c) {
            ops.push(VoxelOp::Set {
                c,
                rgba: neighbor_avg_color(field, c),
            });
        }
    }
    ops
}

/// Smooth: per voxel, occupancy blends toward the 3³ neighbourhood density
/// (`occ' = lerp(occ, density, s·w)`, re-thresholded at ½) — bumps erode, pits
/// fill — and colours box-filter over the same neighbourhood so geometry and
/// paint soften together. Two passes per stamp (one visibly under-smooths at
/// brush scale), run through a private overlay; only the final diff against
/// the input field is emitted.
fn smooth_ops<F: Field>(
    field: &F,
    params: &BrushParams,
    center: VoxelCoord,
    radius: u32,
    s: f32,
) -> Vec<VoxelOp> {
    // (occupied, colour) overrides on top of the base field.
    let mut overlay: HashMap<VoxelCoord, (bool, u32)> = HashMap::new();
    let read = |overlay: &HashMap<VoxelCoord, (bool, u32)>, x: i64, y: i64, z: i64| {
        if x < 0 || y < 0 || z < 0 {
            return (false, 0u32);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // guarded ≥ 0
        let c = VoxelCoord::new(x as u32, y as u32, z as u32);
        overlay
            .get(&c)
            .copied()
            .unwrap_or_else(|| (field.occupied(x, y, z), field.color(x, y, z)))
    };

    for _pass in 0..2 {
        let mut changes: Vec<(VoxelCoord, (bool, u32))> = Vec::new();
        for c in ball(center, radius) {
            let (was, old_color) = read(&overlay, i64::from(c.x), i64::from(c.y), i64::from(c.z));
            let mut count = 0u32;
            let mut csum = [0u32; 3];
            for dz in -1i64..=1 {
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (occ_n, col_n) = read(
                            &overlay,
                            i64::from(c.x) + dx,
                            i64::from(c.y) + dy,
                            i64::from(c.z) + dz,
                        );
                        if occ_n {
                            count += 1;
                            let bytes = col_n.to_le_bytes();
                            for (acc, b) in csum.iter_mut().zip(bytes) {
                                *acc += u32::from(b);
                            }
                        }
                    }
                }
            }
            #[allow(clippy::cast_precision_loss)] // count ≤ 27
            let density = count as f32 / 27.0;
            let sw = (s * weight(params.falloff, t_of(c, center, radius))).clamp(0.0, 1.0);
            let occ_f = if was { 1.0 } else { 0.0 };
            let now = occ_f + (density - occ_f) * sw >= 0.5;
            if now {
                let filtered = if count > 0 {
                    #[allow(clippy::cast_possible_truncation)] // mean of bytes
                    u32::from_le_bytes([
                        (csum[0] / count) as u8,
                        (csum[1] / count) as u8,
                        (csum[2] / count) as u8,
                        255,
                    ])
                } else {
                    old_color
                };
                if !was || filtered != old_color {
                    changes.push((c, (true, filtered)));
                }
            } else if was {
                changes.push((c, (false, 0)));
            }
        }
        for (c, state) in changes {
            overlay.insert(c, state);
        }
    }

    // Emit the diff between the overlay's final state and the base field, in
    // ball order (deterministic — never HashMap order).
    let mut ops = Vec::new();
    for c in ball(center, radius) {
        let Some(&(now, color)) = overlay.get(&c) else {
            continue;
        };
        let (x, y, z) = (i64::from(c.x), i64::from(c.y), i64::from(c.z));
        let was = field.occupied(x, y, z);
        match (was, now) {
            (false, true) => ops.push(VoxelOp::Set { c, rgba: color }),
            (true, false) => ops.push(VoxelOp::Clear { c }),
            (true, true) => {
                if color != field.color(x, y, z) {
                    ops.push(VoxelOp::Recolor { c, rgba: color });
                }
            }
            (false, false) => {}
        }
    }
    ops
}

// --- Colour math (sRGB ↔ linear light) ------------------------------------

/// One sRGB byte → linear light `[0, 1]`.
fn srgb_to_linear(u: u8) -> f32 {
    let c = f32::from(u) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light → an sRGB byte.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to [0,255] first
fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Blends `src` toward `dst` by `t` in **linear light** (sRGB lerp darkens
/// midtones), returning sRGB RGBA8 with a forced-opaque alpha (v1 paint never
/// writes alpha < 255). `src`/`dst` are sRGB RGBA8, R low.
fn blend_srgb(src: u32, dst: u32, t: f32) -> u32 {
    let src_b = src.to_le_bytes();
    let dst_b = dst.to_le_bytes();
    let mut out = [0u8; 4];
    for ch in 0..3 {
        let lin_src = srgb_to_linear(src_b[ch]);
        let lin_dst = srgb_to_linear(dst_b[ch]);
        out[ch] = linear_to_srgb(lin_src + (lin_dst - lin_src) * t);
    }
    out[3] = 255;
    u32::from_le_bytes(out)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::params::Falloff;

    /// A sparse test field over plain collections — no tree in sight.
    #[derive(Default, Clone)]
    struct Grid {
        occ: HashSet<(i64, i64, i64)>,
        colors: HashMap<(i64, i64, i64), u32>,
    }
    impl Grid {
        fn set(&mut self, x: i64, y: i64, z: i64, color: u32) {
            self.occ.insert((x, y, z));
            self.colors.insert((x, y, z), color);
        }
        /// A solid slab `z ∈ [0, top]` over the given xy extent.
        fn slab(extent: i64, top: i64, color: u32) -> Self {
            let mut g = Self::default();
            for x in 0..extent {
                for y in 0..extent {
                    for z in 0..=top {
                        g.set(x, y, z, color);
                    }
                }
            }
            g
        }
        fn apply(&mut self, ops: &[VoxelOp]) {
            for op in ops {
                match *op {
                    VoxelOp::Set { c, rgba } => {
                        self.set(i64::from(c.x), i64::from(c.y), i64::from(c.z), rgba);
                    }
                    VoxelOp::Clear { c } => {
                        let k = (i64::from(c.x), i64::from(c.y), i64::from(c.z));
                        self.occ.remove(&k);
                        self.colors.remove(&k);
                    }
                    VoxelOp::Recolor { c, rgba } => {
                        let k = (i64::from(c.x), i64::from(c.y), i64::from(c.z));
                        assert!(self.occ.contains(&k), "recolor of an empty voxel");
                        self.colors.insert(k, rgba);
                    }
                }
            }
        }
    }
    impl Field for Grid {
        fn occupied(&self, x: i64, y: i64, z: i64) -> bool {
            self.occ.contains(&(x, y, z))
        }
        fn color(&self, x: i64, y: i64, z: i64) -> u32 {
            self.colors.get(&(x, y, z)).copied().unwrap_or(0xFF00_0000)
        }
    }

    fn params(tool: BrushTool, radius: u32, strength: f32) -> BrushParams {
        BrushParams {
            tool,
            radius,
            strength,
            falloff: Falloff::Smooth,
            color: u32::from_le_bytes([200, 40, 40, 255]),
            invert: false,
        }
    }

    fn stamp(x: u32, y: u32, z: u32) -> Stamp {
        Stamp {
            center: VoxelCoord::new(x, y, z),
            pressure: 1.0,
        }
    }

    fn run(field: &Grid, p: &BrushParams, st: &Stamp) -> Vec<VoxelOp> {
        stamp_ops(field, p, st, None, &mut StrokeMask::new())
    }

    #[test]
    fn draw_fills_only_empty_voxels_and_erase_inverts() {
        let g = Grid::slab(16, 2, 0xFF11_2233);
        let p = params(BrushTool::Draw, 2, 1.0);
        let ops = run(&g, &p, &stamp(8, 8, 2));
        assert!(!ops.is_empty());
        for op in &ops {
            let VoxelOp::Set { c, rgba } = *op else {
                panic!("draw emits only Set")
            };
            assert!(!occ(&g, c), "draw never touches occupied voxels");
            assert_eq!(rgba, p.color);
        }
        let mut after = g.clone();
        after.apply(&ops);
        let erase = params(BrushTool::Erase, 2, 1.0);
        let eops = run(&after, &erase, &stamp(8, 8, 2));
        for op in &eops {
            assert!(matches!(op, VoxelOp::Clear { .. }));
        }
    }

    /// Clay: only surface-adjacent voxels change; growth is monotone in
    /// strength; deterministic; new voxels' colour == the neighbour average.
    #[test]
    fn clay_is_surface_biased_monotone_and_colour_inheriting() {
        let mut g = Grid::slab(24, 3, u32::from_le_bytes([10, 20, 30, 255]));
        // A differently-painted patch to make the average observable.
        g.set(12, 12, 3, u32::from_le_bytes([110, 120, 130, 255]));
        let st = stamp(12, 12, 4);

        let weak = run(&g, &params(BrushTool::Clay, 4, 0.6), &st);
        let strong = run(&g, &params(BrushTool::Clay, 4, 1.0), &st);
        assert!(!strong.is_empty());
        let coords =
            |ops: &[VoxelOp]| -> HashSet<VoxelCoord> { ops.iter().map(VoxelOp::coord).collect() };
        assert!(
            coords(&weak).is_subset(&coords(&strong)),
            "growth monotone in strength"
        );
        assert_eq!(strong, run(&g, &params(BrushTool::Clay, 4, 1.0), &st));
        for op in &strong {
            let VoxelOp::Set { c, rgba } = *op else {
                panic!("clay only adds")
            };
            assert!(
                !occ(&g, c) && has_occupied_6_neighbor(&g, c),
                "surface only"
            );
            assert_eq!(rgba, neighbor_avg_color(&g, c), "inherits neighbour avg");
        }
        // The voxel directly above the repainted patch averages over it.
        let above = VoxelCoord::new(12, 12, 4);
        let op = strong
            .iter()
            .find(|o| o.coord() == above)
            .expect("grows up");
        let VoxelOp::Set { rgba, .. } = *op else {
            panic!()
        };
        assert_eq!(rgba, neighbor_avg_color(&g, above));
    }

    /// Inflate: uniform within the strength-scaled reach — every empty
    /// surface-adjacent voxel with `t ≤ s` sets, regardless of falloff.
    #[test]
    fn inflate_dilates_uniformly_within_reach() {
        let g = Grid::slab(24, 3, 0xFF33_3333);
        let st = stamp(12, 12, 3);
        let p = params(BrushTool::Inflate, 5, 1.0);
        let ops = run(&g, &p, &st);
        let got: HashSet<VoxelCoord> = ops.iter().map(VoxelOp::coord).collect();
        for c in brush_voxels(st.center, 5) {
            let expect =
                !occ(&g, c) && has_occupied_6_neighbor(&g, c) && t_of(c, st.center, 5) <= 1.0;
            assert_eq!(got.contains(&c), expect, "{c:?}");
        }
        // Half strength halves the reach.
        let half = run(&g, &params(BrushTool::Inflate, 5, 0.5), &st);
        for op in &half {
            assert!(t_of(op.coord(), st.center, 5) <= 0.5 + 1e-6);
        }
    }

    /// Deflate (Inflate + invert): only exposed surface-shell voxels clear,
    /// uniformly within the strength-scaled reach; interior stays solid.
    #[test]
    fn deflate_thins_only_the_exposed_shell() {
        let g = Grid::slab(24, 5, 0xFF66_6666);
        let st = stamp(12, 12, 5);
        let p = BrushParams {
            invert: true,
            ..params(BrushTool::Inflate, 4, 1.0)
        };
        let ops = run(&g, &p, &st);
        assert!(!ops.is_empty());
        for op in &ops {
            let VoxelOp::Clear { c } = *op else {
                panic!("deflate only clears")
            };
            assert!(
                occ(&g, c) && has_empty_6_neighbor(&g, c),
                "shell only: {c:?}"
            );
        }
        // Interior voxels under the shell survive.
        let mut after = g.clone();
        after.apply(&ops);
        assert!(after.occupied(12, 12, 3), "interior must survive");
    }

    /// The pressure response curve: light pen pressure shrinks the effective
    /// stamp radius (half size at zero), a mouse's 1.0 is the identity.
    #[test]
    fn pressure_scales_the_effective_radius() {
        assert_eq!(effective_radius(8, 1.0), 8);
        assert_eq!(effective_radius(8, 0.5), 6);
        assert_eq!(effective_radius(8, 0.0), 4);
        assert_eq!(effective_radius(1, 0.0), 1, "never below one voxel");
        let g = Grid::slab(24, 3, 0xFF11_1111);
        let p = params(BrushTool::Draw, 6, 1.0);
        let light = Stamp {
            center: VoxelCoord::new(12, 12, 3),
            pressure: 0.2,
        };
        let full = Stamp {
            center: VoxelCoord::new(12, 12, 3),
            pressure: 1.0,
        };
        let light_ops = stamp_ops(&g, &p, &light, None, &mut StrokeMask::new());
        let full_ops = stamp_ops(&g, &p, &full, None, &mut StrokeMask::new());
        assert!(
            light_ops.len() < full_ops.len(),
            "light touch marks smaller"
        );
    }

    /// Flatten post-conditions: nothing occupied ends more than half a voxel
    /// above the anchor plane; nothing below the slab floor is removed; fills
    /// have support; and without an anchor the kernel is silent.
    #[test]
    fn flatten_clamps_to_the_anchor_plane() {
        let mut g = Grid::slab(24, 4, 0xFF77_7777);
        for (bx, by) in [(10, 10), (11, 10), (10, 11)] {
            for bz in 5..=7 {
                g.set(bx, by, bz, 0xFF77_7777); // a bump above the plane
            }
        }
        let anchor = Plane {
            point: glam::DVec3::new(10.0, 10.0, 4.0),
            normal: glam::DVec3::Z,
        };
        let flat = params(BrushTool::Flatten, 5, 1.0);
        let st = stamp(10, 10, 5);
        let ops = stamp_ops(&g, &flat, &st, Some(anchor), &mut StrokeMask::new());
        let mut after = g.clone();
        after.apply(&ops);
        for c in brush_voxels(st.center, 5) {
            let probe = (i64::from(c.x), i64::from(c.y), i64::from(c.z));
            let signed = f64::from(c.z) - 4.0;
            if after.occupied(probe.0, probe.1, probe.2) {
                assert!(
                    signed <= 0.5,
                    "occupied voxel {c:?} survives above the plane"
                );
            }
            if g.occupied(probe.0, probe.1, probe.2) && signed < -1.25 {
                assert!(
                    after.occupied(probe.0, probe.1, probe.2),
                    "material below the slab floor removed"
                );
            }
        }
        for op in &ops {
            if let VoxelOp::Set { c, .. } = *op {
                assert!(has_occupied_6_neighbor(&g, c), "fill without support");
            }
        }
        assert!(
            stamp_ops(&g, &flat, &st, None, &mut StrokeMask::new()).is_empty(),
            "no anchor, no ops"
        );
    }

    /// The Smooth oracle: the kernel through the `Field` trait must equal an
    /// independent brute-force implementation over a dense bit-grid (same
    /// two-pass definition, plain arrays, no overlay machinery).
    #[test]
    fn smooth_matches_a_dense_brute_force_reference() {
        // A noisy step: a slab with a deterministic scatter of bumps and pits.
        let mut g = Grid::slab(20, 3, 0xFF40_4040);
        for i in 0..20i64 {
            let sx = (i * 7) % 20;
            let sy = (i * 11) % 20;
            if i % 3 == 0 {
                g.set(sx, sy, 4, 0xFF90_9090); // bump
            } else {
                let key = (sx, sy, 3);
                g.occ.remove(&key); // pit
                g.colors.remove(&key);
            }
        }
        let smoothp = params(BrushTool::Smooth, 4, 1.0);
        let st = stamp(10, 10, 3);
        let mut after = g.clone();
        after.apply(&run(&g, &smoothp, &st));

        // Reference: dense arrays over the bounding box, two passes.
        let ext = 26i64;
        let idx = |ix: i64, iy: i64, iz: i64| ((iz * ext + iy) * ext + ix) as usize;
        let mut dense = vec![false; (ext * ext * ext) as usize];
        for &(ox, oy, oz) in &g.occ {
            if ox < ext && oy < ext && oz < ext {
                dense[idx(ox, oy, oz)] = true;
            }
        }
        let in_ball: Vec<VoxelCoord> = brush_voxels(st.center, 4);
        for _pass in 0..2 {
            let mut next = dense.clone();
            for &c in &in_ball {
                let (cx, cy, cz) = (i64::from(c.x), i64::from(c.y), i64::from(c.z));
                let mut count = 0u32;
                for dz in -1i64..=1 {
                    for dy in -1i64..=1 {
                        for dx in -1i64..=1 {
                            let (px, py, pz) = (cx + dx, cy + dy, cz + dz);
                            if (0..ext).contains(&px)
                                && (0..ext).contains(&py)
                                && (0..ext).contains(&pz)
                                && dense[idx(px, py, pz)]
                            {
                                count += 1;
                            }
                        }
                    }
                }
                #[allow(clippy::cast_precision_loss)]
                let density = count as f32 / 27.0;
                let sw = weight(smoothp.falloff, t_of(c, st.center, 4));
                let occ_f = if dense[idx(cx, cy, cz)] { 1.0 } else { 0.0 };
                next[idx(cx, cy, cz)] = occ_f + (density - occ_f) * sw >= 0.5;
            }
            dense = next;
        }
        for &c in &in_ball {
            let (qx, qy, qz) = (i64::from(c.x), i64::from(c.y), i64::from(c.z));
            assert_eq!(
                after.occupied(qx, qy, qz),
                dense[idx(qx, qy, qz)],
                "occupancy diverges at {c:?}"
            );
        }
    }

    /// Paint + `StrokeMask`: overlapping stamps within one stroke apply at
    /// most the single max alpha; blending is linear-light, exact endpoints.
    #[test]
    fn paint_masks_overlaps_and_blends_in_linear_light() {
        let g = Grid::slab(16, 2, u32::from_le_bytes([0, 0, 0, 255]));
        let p = params(BrushTool::Paint, 3, 1.0);
        let st = stamp(8, 8, 2);
        let mut mask = StrokeMask::new();
        let first = stamp_ops(&g, &p, &st, None, &mut mask);
        assert!(!first.is_empty());
        let mut after = g.clone();
        after.apply(&first);
        // The identical stamp again, same stroke: every voxel already carries
        // its max alpha, so nothing new is emitted.
        let second = stamp_ops(&after, &p, &st, None, &mut mask);
        assert!(second.is_empty(), "re-stamp at equal alpha is a no-op");

        // Blend endpoints against the byte space directly.
        let red = u32::from_le_bytes([255, 0, 0, 255]);
        let blue = u32::from_le_bytes([0, 0, 255, 255]);
        assert_eq!(blend_srgb(red, blue, 0.0), red);
        assert_eq!(blend_srgb(red, blue, 1.0), blue);
        // Linear-light midpoint of black→white is ~0.5 linear ≈ 188 sRGB —
        // provably not the naive byte lerp (128).
        let black = u32::from_le_bytes([0, 0, 0, 255]);
        let white = u32::from_le_bytes([255, 255, 255, 255]);
        let mid = blend_srgb(black, white, 0.5).to_le_bytes();
        assert_eq!(mid[0], linear_to_srgb(0.5));
        assert!(mid[0] > 150, "linear-light midpoint, not byte lerp");
        // Alpha stays opaque even blending toward a transparent brush.
        let clear = u32::from_le_bytes([0, 255, 0, 0]);
        assert_eq!(blend_srgb(red, clear, 0.5).to_le_bytes()[3], 255);
    }
}
