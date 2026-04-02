//! CPU voxelizer — SAT-based triangle-box overlap, outputs OccupancyBuilder + PaletteBuilder.
//!
//! This is the initial path for Phase 2 (OBJ → pool). It serves as both the
//! working implementation and the test oracle for the future GPU voxelizer.
//!
//! Algorithm ported from `crates/voxelizer/src/reference_cpu.rs`.

use std::collections::HashMap;

use glam::Vec3;

use crate::obj_parser::ParsedObj;
use crate::pool::*;
use crate::scene::{ChunkData, IndexBufBuilder, MaterialEntry, OccupancyBuilder, PaletteBuilder};

/// Result of voxelization: chunk data ready for upload + material table.
pub struct VoxelizeResult {
    pub chunks: Vec<ChunkData>,
    pub materials: Vec<MaterialEntry>,
}

/// Voxelize a parsed OBJ mesh into chunk data.
///
/// `resolution` controls voxels along the mesh's longest axis.
/// Returns chunk data and a material table ready for upload.
pub fn voxelize(parsed: &ParsedObj, resolution: u32) -> VoxelizeResult {
    let resolution = resolution.max(1);

    // Compute mesh AABB
    let (mesh_min, mesh_max) = mesh_aabb(parsed);
    let extent = mesh_max - mesh_min;
    let longest = extent.x.max(extent.y).max(extent.z);

    if longest <= 0.0 || parsed.triangles.is_empty() {
        return VoxelizeResult {
            chunks: Vec::new(),
            materials: build_material_table(parsed),
        };
    }

    let voxel_size = longest / resolution as f32;
    // Grid origin: offset so mesh sits at (0,0,0) in grid space with a small margin
    let margin = Vec3::splat(voxel_size * 0.5);
    let grid_origin = mesh_min - margin;

    // Grid dimensions in voxels
    let grid_dims = [
        ((extent.x + margin.x * 2.0) / voxel_size).ceil() as u32,
        ((extent.y + margin.y * 2.0) / voxel_size).ceil() as u32,
        ((extent.z + margin.z * 2.0) / voxel_size).ceil() as u32,
    ];

    // Per-chunk accumulators keyed by chunk coord
    let mut chunk_map: HashMap<(i32, i32, i32), ChunkAccum> = HashMap::new();

    let half = Vec3::splat(0.5);
    let epsilon = 0.01_f32;

    for (tri_idx, tri) in parsed.triangles.iter().enumerate() {
        // Transform triangle to grid space
        let v0 = (pos(parsed, tri[0]) - grid_origin) / voxel_size;
        let v1 = (pos(parsed, tri[1]) - grid_origin) / voxel_size;
        let v2 = (pos(parsed, tri[2]) - grid_origin) / voxel_size;

        // Triangle AABB with epsilon expansion
        let tri_min = v0.min(v1).min(v2) - Vec3::splat(epsilon);
        let tri_max = v0.max(v1).max(v2) + Vec3::splat(epsilon);

        let min_x = (tri_min.x.floor() as i32).max(0);
        let min_y = (tri_min.y.floor() as i32).max(0);
        let min_z = (tri_min.z.floor() as i32).max(0);
        let max_x = (tri_max.x.floor() as i32).min(grid_dims[0] as i32 - 1);
        let max_y = (tri_max.y.floor() as i32).min(grid_dims[1] as i32 - 1);
        let max_z = (tri_max.z.floor() as i32).min(grid_dims[2] as i32 - 1);

        let mat_group = parsed.triangle_materials[tri_idx] as u16;

        for gz in min_z..=max_z {
            for gy in min_y..=max_y {
                for gx in min_x..=max_x {
                    let center = Vec3::new(gx as f32 + 0.5, gy as f32 + 0.5, gz as f32 + 0.5);
                    if triangle_box_overlap(center, half, v0, v1, v2) {
                        // Compute chunk coord and local position
                        let cx = floor_div(gx, CS as i32);
                        let cy = floor_div(gy, CS as i32);
                        let cz = floor_div(gz, CS as i32);

                        let lx = (euclidean_mod(gx, CS as i32) + 1) as u32;
                        let ly = (euclidean_mod(gy, CS as i32) + 1) as u32;
                        let lz = (euclidean_mod(gz, CS as i32) + 1) as u32;

                        let accum = chunk_map
                            .entry((cx, cy, cz))
                            .or_insert_with(ChunkAccum::new);
                        accum.occupancy.set(lx, ly, lz);

                        // Material: +2 offset (0=MATERIAL_EMPTY, 1=MATERIAL_DEFAULT)
                        let mat_id = mat_group + 2;
                        accum.set_material(lx, ly, lz, mat_id);
                    }
                }
            }
        }
    }

    // Convert accumulators to ChunkData
    let mut chunks: Vec<ChunkData> = chunk_map
        .into_iter()
        .map(|((cx, cy, cz), accum)| accum.into_chunk_data(ChunkCoord { x: cx, y: cy, z: cz }))
        .collect();

    // Sort by coord for deterministic output
    chunks.sort_by_key(|c| (c.coord.x, c.coord.y, c.coord.z));

    VoxelizeResult {
        chunks,
        materials: build_material_table(parsed),
    }
}

// ─── Internals ─────────────────────────────────────────────────────────────

/// Per-chunk accumulator during voxelization.
struct ChunkAccum {
    occupancy: OccupancyBuilder,
    /// Per-voxel material ID (only for occupied voxels).
    /// Key: (lx, ly, lz) packed as u32.
    voxel_materials: HashMap<u32, u16>,
}

impl ChunkAccum {
    fn new() -> Self {
        Self {
            occupancy: OccupancyBuilder::new(),
            voxel_materials: HashMap::new(),
        }
    }

    fn set_material(&mut self, lx: u32, ly: u32, lz: u32, mat_id: u16) {
        let key = lx * CS_P * CS_P + ly * CS_P + lz;
        self.voxel_materials.entry(key).or_insert(mat_id);
    }

    fn into_chunk_data(self, coord: ChunkCoord) -> ChunkData {
        let mut palette = PaletteBuilder::new();
        let mut index_buf = IndexBufBuilder::new();

        // Scan all occupied voxels, build palette + per-voxel index assignments
        for (&key, &mat_id) in &self.voxel_materials {
            let lx = key / (CS_P * CS_P);
            let ly = (key / CS_P) % CS_P;
            let lz = key % CS_P;
            let palette_idx = palette.add(mat_id);
            index_buf.set(lx, ly, lz, palette_idx);
        }

        ChunkData {
            coord,
            occupancy: self.occupancy,
            palette,
            index_buf,
        }
    }
}

/// Get vertex position as Vec3.
fn pos(parsed: &ParsedObj, idx: u32) -> Vec3 {
    let p = parsed.positions[idx as usize];
    Vec3::new(p[0], p[1], p[2])
}

/// Compute mesh AABB.
fn mesh_aabb(parsed: &ParsedObj) -> (Vec3, Vec3) {
    if parsed.positions.is_empty() {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for p in &parsed.positions {
        let v = Vec3::new(p[0], p[1], p[2]);
        min = min.min(v);
        max = max.max(v);
    }
    (min, max)
}

/// Build material table from parsed OBJ material groups.
/// Material group i in the OBJ maps to MaterialId (i + 2) in the table.
/// IDs 0 and 1 are reserved for MATERIAL_EMPTY and MATERIAL_DEFAULT.
fn build_material_table(parsed: &ParsedObj) -> Vec<MaterialEntry> {
    let mut table = vec![
        MaterialEntry::new([0.0; 3], 0.0, [0.0; 3], 0.0);
        MAX_MATERIALS as usize
    ];

    // 0: empty (zeroed)
    // 1: default gray
    table[MATERIAL_DEFAULT as usize] =
        MaterialEntry::new([0.5, 0.5, 0.5], 0.5, [0.0; 3], 1.0);

    // Assign deterministic colors to material groups
    for (i, _name) in parsed.material_names.iter().enumerate() {
        let mat_id = (i as u16) + 2;
        if (mat_id as usize) < table.len() {
            let color = hash_color_f32(i as u32);
            table[mat_id as usize] =
                MaterialEntry::new(color, 0.5, [0.0; 3], 1.0);
        }
    }

    table
}

/// Deterministic color from an integer — produces visually distinct hues.
fn hash_color_f32(id: u32) -> [f32; 3] {
    let mut x = id.wrapping_mul(1664525).wrapping_add(1013904223);
    let r = (x & 0xFF) as f32 / 255.0;
    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
    let g = (x & 0xFF) as f32 / 255.0;
    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
    let b = (x & 0xFF) as f32 / 255.0;
    // Boost saturation: mix with 0.3 to avoid too-dark or too-light colors
    [0.3 + r * 0.7, 0.3 + g * 0.7, 0.3 + b * 0.7]
}

/// Euclidean floor division (works for negative dividends).
fn floor_div(a: i32, b: i32) -> i32 {
    let d = a / b;
    let r = a % b;
    if (r != 0) && ((r ^ b) < 0) { d - 1 } else { d }
}

/// Euclidean modulus (always non-negative).
fn euclidean_mod(a: i32, b: i32) -> i32 {
    let r = a % b;
    if r < 0 { r + b } else { r }
}

// ─── SAT triangle-box overlap ──────────────────────────────────────────────
// Ported from crates/voxelizer/src/reference_cpu.rs

/// SAT (Separating Axis Theorem) conservative triangle-box overlap test.
/// Tests 9 edge cross-product axes + 3 AABB face axes + 1 triangle plane.
fn triangle_box_overlap(box_center: Vec3, box_half: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> bool {
    let v0 = v0 - box_center;
    let v1 = v1 - box_center;
    let v2 = v2 - box_center;

    let e0 = v1 - v0;
    let e1 = v2 - v1;
    let e2 = v0 - v2;

    // 9 edge cross-product axes
    let axes = [
        Vec3::new(0.0, -e0.z, e0.y),
        Vec3::new(0.0, -e1.z, e1.y),
        Vec3::new(0.0, -e2.z, e2.y),
        Vec3::new(e0.z, 0.0, -e0.x),
        Vec3::new(e1.z, 0.0, -e1.x),
        Vec3::new(e2.z, 0.0, -e2.x),
        Vec3::new(-e0.y, e0.x, 0.0),
        Vec3::new(-e1.y, e1.x, 0.0),
        Vec3::new(-e2.y, e2.x, 0.0),
    ];

    for axis in axes.iter() {
        let p0 = v0.dot(*axis);
        let p1 = v1.dot(*axis);
        let p2 = v2.dot(*axis);
        let min_p = p0.min(p1.min(p2));
        let max_p = p0.max(p1.max(p2));
        let r = box_half.x * axis.x.abs()
            + box_half.y * axis.y.abs()
            + box_half.z * axis.z.abs();
        if min_p > r || max_p < -r {
            return false;
        }
    }

    // 3 AABB face axes (box vs triangle AABB)
    if v0.x.min(v1.x.min(v2.x)) > box_half.x
        || v0.x.max(v1.x.max(v2.x)) < -box_half.x
        || v0.y.min(v1.y.min(v2.y)) > box_half.y
        || v0.y.max(v1.y.max(v2.y)) < -box_half.y
        || v0.z.min(v1.z.min(v2.z)) > box_half.z
        || v0.z.max(v1.z.max(v2.z)) < -box_half.z
    {
        return false;
    }

    // Triangle plane test
    let normal = e0.cross(e1);
    let d = -normal.dot(v0);
    let r = box_half.x * normal.x.abs()
        + box_half.y * normal.y.abs()
        + box_half.z * normal.z.abs();
    let s = d; // normal.dot(Vec3::ZERO) + d = d
    if s.abs() > r {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_triangle_obj(v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> ParsedObj {
        ParsedObj {
            positions: vec![v0, v1, v2],
            triangles: vec![[0, 1, 2]],
            triangle_materials: vec![0],
            material_names: vec!["(default)".to_string()],
        }
    }

    #[test]
    fn voxelize_empty_mesh() {
        let parsed = ParsedObj {
            positions: Vec::new(),
            triangles: Vec::new(),
            triangle_materials: Vec::new(),
            material_names: vec!["(default)".to_string()],
        };
        let result = voxelize(&parsed, 62);
        assert!(result.chunks.is_empty());
    }

    #[test]
    fn voxelize_single_triangle() {
        let parsed = make_triangle_obj(
            [0.0, 0.0, 0.0],
            [5.0, 0.0, 0.0],
            [0.0, 5.0, 0.0],
        );
        let result = voxelize(&parsed, 10);
        assert!(!result.chunks.is_empty());
        let total: u32 = result.chunks.iter().map(|c| c.occupancy.popcount()).sum();
        assert!(total > 0, "expected occupied voxels");
    }

    #[test]
    fn voxelize_cube_mesh() {
        // Unit cube from 0 to 1
        let parsed = ParsedObj {
            positions: vec![
                [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0],
            ],
            triangles: vec![
                // Front
                [0, 1, 2], [0, 2, 3],
                // Back
                [4, 7, 6], [4, 6, 5],
                // Bottom
                [0, 4, 5], [0, 5, 1],
                // Top
                [3, 2, 6], [3, 6, 7],
                // Right
                [1, 5, 6], [1, 6, 2],
                // Left
                [0, 3, 7], [0, 7, 4],
            ],
            triangle_materials: vec![0; 12],
            material_names: vec!["(default)".to_string()],
        };
        let result = voxelize(&parsed, 10);
        let total: u32 = result.chunks.iter().map(|c| c.occupancy.popcount()).sum();
        // A 10-voxel cube surface should have many voxels
        assert!(total >= 10, "expected >= 10 surface voxels, got {total}");
    }

    #[test]
    fn voxelize_multi_material() {
        let parsed = ParsedObj {
            positions: vec![
                [0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0],
                [3.0, 0.0, 0.0], [5.0, 0.0, 0.0], [3.0, 2.0, 0.0],
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            triangle_materials: vec![0, 1],
            material_names: vec!["red".to_string(), "blue".to_string()],
        };
        let result = voxelize(&parsed, 10);
        assert!(!result.chunks.is_empty());
        // Material table should have entries for (default), red (id=2), blue (id=3)
        assert_ne!(
            result.materials[2].albedo_rg,
            result.materials[3].albedo_rg,
            "different materials should have different colors"
        );
    }

    #[test]
    fn voxelize_single_chunk_bounds() {
        // Small mesh that fits in one chunk (resolution=10)
        let parsed = make_triangle_obj(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
        let result = voxelize(&parsed, 10);
        assert_eq!(result.chunks.len(), 1, "small mesh should fit in one chunk");
    }

    #[test]
    fn voxelize_local_coords_in_usable_range() {
        // Verify all occupied voxels are in the usable interior [1, 62]
        let parsed = make_triangle_obj(
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [0.0, 3.0, 0.0],
        );
        let result = voxelize(&parsed, 20);
        for chunk in &result.chunks {
            for x in 0..CS_P {
                for z in 0..CS_P {
                    for y in 0..CS_P {
                        if chunk.occupancy.get(x, y, z) {
                            assert!(
                                x >= 1 && x <= CS && z >= 1 && z <= CS && y >= 1 && y <= CS,
                                "voxel at ({x},{y},{z}) is outside usable range [1,{CS}]"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sat_overlap_basic() {
        let center = Vec3::new(0.5, 0.5, 0.5);
        let half = Vec3::splat(0.5);
        // Triangle passing through the unit cube
        assert!(triangle_box_overlap(
            center, half,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ));
    }

    #[test]
    fn sat_overlap_miss() {
        let center = Vec3::new(0.5, 0.5, 0.5);
        let half = Vec3::splat(0.5);
        // Triangle far away
        assert!(!triangle_box_overlap(
            center, half,
            Vec3::new(10.0, 10.0, 10.0),
            Vec3::new(11.0, 10.0, 10.0),
            Vec3::new(10.0, 11.0, 10.0),
        ));
    }

    #[test]
    fn floor_div_positive() {
        assert_eq!(floor_div(7, 3), 2);
        assert_eq!(floor_div(6, 3), 2);
    }

    #[test]
    fn floor_div_negative() {
        assert_eq!(floor_div(-1, 62), -1);
        assert_eq!(floor_div(-62, 62), -1);
        assert_eq!(floor_div(-63, 62), -2);
    }

    #[test]
    fn euclidean_mod_positive() {
        assert_eq!(euclidean_mod(7, 3), 1);
    }

    #[test]
    fn euclidean_mod_negative() {
        assert_eq!(euclidean_mod(-1, 62), 61);
        assert_eq!(euclidean_mod(-62, 62), 0);
        assert_eq!(euclidean_mod(-63, 62), 61);
    }
}
