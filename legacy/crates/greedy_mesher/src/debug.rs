//! Debug geometry generation for greedy mesh visualization.
//!
//! Generates wireframe lines showing quad boundaries and per-vertex colors
//! for merge efficiency visualization.

use crate::core::{
    unpack_quad,
    FACE_POS_Y, FACE_NEG_Y, FACE_POS_X, FACE_NEG_X, FACE_POS_Z, FACE_NEG_Z,
};

/// Debug output containing wireframe lines and per-vertex colors.
pub struct DebugGeometry {
    /// Wireframe line positions (pairs of xyz endpoints).
    /// Length = quad_count * 4 edges * 2 endpoints * 3 floats = quad_count * 24.
    pub line_positions: Vec<f32>,

    /// Per-vertex face-direction colors for the main mesh (RGB per vertex).
    /// Length = vertex_count * 3.
    pub face_colors: Vec<f32>,

    /// Per-vertex quad-size colors for the main mesh (RGB per vertex).
    /// Heatmap: small quads = red, large quads = green.
    /// Length = vertex_count * 3.
    pub size_colors: Vec<f32>,
}

/// Face direction colors (matching the plan spec).
const DIR_COLORS: [[f32; 3]; 6] = [
    [0.2, 0.9, 0.2],  // +Y = green
    [0.1, 0.5, 0.1],  // -Y = dark green
    [0.9, 0.2, 0.2],  // +X = red
    [0.5, 0.1, 0.1],  // -X = dark red
    [0.2, 0.2, 0.9],  // +Z = blue
    [0.1, 0.1, 0.5],  // -Z = dark blue
];

/// Compute 4 corner positions for a quad (must match expand.rs logic).
fn quad_corners(
    face: usize,
    x: u32, y: u32, z: u32,
    width: u32, height: u32,
    voxel_size: f32,
    origin: [f32; 3],
) -> [[f32; 3]; 4] {
    let bx = origin[0] + x as f32 * voxel_size;
    let by = origin[1] + y as f32 * voxel_size;
    let bz = origin[2] + z as f32 * voxel_size;
    let w = width as f32 * voxel_size;
    let h = height as f32 * voxel_size;

    match face {
        FACE_POS_Y => [
            [bx, by + voxel_size, bz],
            [bx + w, by + voxel_size, bz],
            [bx + w, by + voxel_size, bz + h],
            [bx, by + voxel_size, bz + h],
        ],
        FACE_NEG_Y => [
            [bx, by, bz],
            [bx, by, bz + h],
            [bx + w, by, bz + h],
            [bx + w, by, bz],
        ],
        // X-faces: width extends along Y, height extends along Z
        FACE_POS_X => [
            [bx + voxel_size, by, bz],
            [bx + voxel_size, by + w, bz],
            [bx + voxel_size, by + w, bz + h],
            [bx + voxel_size, by, bz + h],
        ],
        FACE_NEG_X => [
            [bx, by, bz],
            [bx, by, bz + h],
            [bx, by + w, bz + h],
            [bx, by + w, bz],
        ],
        FACE_POS_Z => [
            [bx, by, bz + voxel_size],
            [bx + w, by, bz + voxel_size],
            [bx + w, by + h, bz + voxel_size],
            [bx, by + h, bz + voxel_size],
        ],
        FACE_NEG_Z => [
            [bx, by, bz],
            [bx, by + h, bz],
            [bx + w, by + h, bz],
            [bx + w, by, bz],
        ],
        _ => unreachable!(),
    }
}

/// Size-based heatmap color: small quads are red, large quads are green.
///
/// Uses a logarithmic scale so that the practically relevant range
/// (1–100 area) spans most of the gradient, rather than compressing
/// everything into the red end of a linear 1–3844 scale.
fn size_color(width: u32, height: u32) -> [f32; 3] {
    let area = (width * height) as f32;
    // Log scale: ln(1)=0, ln(3844)≈8.25
    let max_log = (62.0_f32 * 62.0).ln();
    let t = (area.ln() / max_log).clamp(0.0, 1.0);
    // Red → Yellow → Green
    let r = (1.0 - t).clamp(0.0, 1.0);
    let g = t.clamp(0.0, 1.0);
    [r, g, 0.1]
}

/// Generate debug geometry from packed quads.
///
/// The wireframe lines show quad boundaries. The color arrays can be applied
/// to the main mesh for face-direction or merge-size visualization.
///
/// # Arguments
/// * `packed_quads` - Array of 6 quad vectors, one per face direction
/// * `voxel_size` - Size of each voxel in world units
/// * `origin` - World position offset for the chunk
pub fn generate_debug_geometry(
    packed_quads: &[Vec<u64>; 6],
    voxel_size: f32,
    origin: [f32; 3],
) -> DebugGeometry {
    let total_quads: usize = packed_quads.iter().map(|q| q.len()).sum();

    // Wireframe: 4 edges per quad, 2 endpoints per edge, 3 floats per endpoint
    let mut line_positions = Vec::with_capacity(total_quads * 24);

    // Colors: 4 vertices per quad, 3 floats per vertex
    let mut face_colors = Vec::with_capacity(total_quads * 12);
    let mut size_colors = Vec::with_capacity(total_quads * 12);

    for (face, quads) in packed_quads.iter().enumerate() {
        let dir_color = DIR_COLORS[face];

        for &quad in quads {
            let (x, y, z, w, h, _material) = unpack_quad(quad);
            let corners = quad_corners(face, x, y, z, w, h, voxel_size, origin);

            // Wireframe edges: 0→1, 1→2, 2→3, 3→0
            for i in 0..4 {
                let a = corners[i];
                let b = corners[(i + 1) % 4];
                line_positions.extend_from_slice(&a);
                line_positions.extend_from_slice(&b);
            }

            // Per-vertex colors (4 vertices per quad)
            let sc = size_color(w, h);
            for _ in 0..4 {
                face_colors.extend_from_slice(&dir_color);
                size_colors.extend_from_slice(&sc);
            }
        }
    }

    DebugGeometry {
        line_positions,
        face_colors,
        size_colors,
    }
}

/// Per-direction face statistics.
#[derive(Debug, Clone, Default)]
pub struct FaceDirectionStats {
    /// Face counts per direction: [+Y, -Y, +X, -X, +Z, -Z]
    pub face_counts: [usize; 6],
    /// Quad counts per direction: [+Y, -Y, +X, -X, +Z, -Z]
    pub quad_counts: [usize; 6],
    /// Total faces before merging
    pub total_faces: usize,
    /// Total quads after merging
    pub total_quads: usize,
    /// Naive triangle count (2 per face, no merging)
    pub naive_triangles: usize,
    /// Merged triangle count (2 per quad)
    pub merged_triangles: usize,
    /// Triangle reduction ratio (naive / merged)
    pub triangle_reduction: f32,
}

/// Compute per-direction face statistics from packed quads and face masks.
pub fn compute_direction_stats(
    packed_quads: &[Vec<u64>; 6],
    max_possible_quads: usize,
) -> FaceDirectionStats {
    let mut stats = FaceDirectionStats::default();

    for (face, quads) in packed_quads.iter().enumerate() {
        stats.quad_counts[face] = quads.len();

        // Count faces covered by quads in this direction
        let face_count: usize = quads.iter().map(|&q| {
            let (_x, _y, _z, w, h, _mat) = unpack_quad(q);
            (w as usize) * (h as usize)
        }).sum();
        stats.face_counts[face] = face_count;
    }

    stats.total_faces = max_possible_quads;
    stats.total_quads = stats.quad_counts.iter().sum();
    stats.naive_triangles = stats.total_faces * 2;
    stats.merged_triangles = stats.total_quads * 2;
    stats.triangle_reduction = if stats.merged_triangles > 0 {
        stats.naive_triangles as f32 / stats.merged_triangles as f32
    } else {
        0.0
    };

    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pack_quad;

    #[test]
    fn wireframe_single_quad() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(10, 10, 10, 5, 3, 1));

        let debug = generate_debug_geometry(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

        // 1 quad → 4 edges → 8 endpoints → 24 floats
        assert_eq!(debug.line_positions.len(), 24);
        // 1 quad → 4 vertices → 12 floats per color array
        assert_eq!(debug.face_colors.len(), 12);
        assert_eq!(debug.size_colors.len(), 12);
    }

    #[test]
    fn wireframe_multiple_faces() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(0, 0, 0, 1, 1, 1));
        packed_quads[FACE_NEG_Y].push(pack_quad(0, 0, 0, 1, 1, 1));
        packed_quads[FACE_POS_X].push(pack_quad(0, 0, 0, 1, 1, 1));

        let debug = generate_debug_geometry(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

        // 3 quads → 72 floats for lines, 36 for colors
        assert_eq!(debug.line_positions.len(), 72);
        assert_eq!(debug.face_colors.len(), 36);
    }

    #[test]
    fn face_colors_match_direction() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        packed_quads[FACE_POS_Y].push(pack_quad(0, 0, 0, 1, 1, 1));

        let debug = generate_debug_geometry(&packed_quads, 1.0, [0.0, 0.0, 0.0]);

        // All 4 vertices should have +Y color (green)
        for i in 0..4 {
            assert_eq!(debug.face_colors[i * 3], DIR_COLORS[FACE_POS_Y][0]);
            assert_eq!(debug.face_colors[i * 3 + 1], DIR_COLORS[FACE_POS_Y][1]);
            assert_eq!(debug.face_colors[i * 3 + 2], DIR_COLORS[FACE_POS_Y][2]);
        }
    }

    #[test]
    fn size_color_small_is_red() {
        let c = size_color(1, 1);
        assert!(c[0] > 0.9); // red
        assert!(c[1] < 0.1); // not green
    }

    #[test]
    fn size_color_large_is_green() {
        let c = size_color(62, 62);
        assert!(c[0] < 0.1); // not red
        assert!(c[1] > 0.9); // green
    }

    #[test]
    fn direction_stats() {
        let mut packed_quads: [Vec<u64>; 6] = Default::default();
        // +Y: one 10x10 quad (100 faces)
        packed_quads[FACE_POS_Y].push(pack_quad(0, 0, 0, 10, 10, 1));
        // -Y: two quads (5x5 = 25 each = 50 total)
        packed_quads[FACE_NEG_Y].push(pack_quad(0, 0, 0, 5, 5, 1));
        packed_quads[FACE_NEG_Y].push(pack_quad(5, 0, 0, 5, 5, 1));

        let stats = compute_direction_stats(&packed_quads, 150);

        assert_eq!(stats.quad_counts[FACE_POS_Y], 1);
        assert_eq!(stats.quad_counts[FACE_NEG_Y], 2);
        assert_eq!(stats.face_counts[FACE_POS_Y], 100);
        assert_eq!(stats.face_counts[FACE_NEG_Y], 50);
        assert_eq!(stats.total_quads, 3);
        assert_eq!(stats.total_faces, 150);
        assert_eq!(stats.naive_triangles, 300);
        assert_eq!(stats.merged_triangles, 6);
        assert!(stats.triangle_reduction > 49.0);
    }
}
