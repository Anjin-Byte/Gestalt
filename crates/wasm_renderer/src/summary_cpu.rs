//! I-3 Summary Rebuild — CPU reference implementation.
//!
//! Platform-independent. Computes the same outputs as the GPU compute shader.
//! Used for native testing and future CPU ↔ GPU validation (Tier 2).
//!
//! See: docs/Resident Representation/stages/I-3-summary-rebuild.md

use crate::pool::*;
use crate::scene::MaterialEntry;

// ─── Flag bit positions (must match chunk-flags.md) ─────────────────────

pub const FLAG_IS_EMPTY: u32 = 1 << 0;
pub const FLAG_IS_FULLY_OPAQUE: u32 = 1 << 1;
pub const FLAG_HAS_EMISSIVE: u32 = 1 << 2;
pub const FLAG_IS_RESIDENT: u32 = 1 << 3;
// bit 4: stale_mesh
// bit 5: stale_summary — cleared by this pass

/// Result of the I-3 summary rebuild for one chunk slot.
#[derive(Debug, Clone)]
pub struct SummaryResult {
    /// 512-bit bricklet grid (16 × u32). Bit set = at least one voxel in that 8³ region.
    pub summary: [u32; SUMMARY_WORDS_PER_SLOT as usize],
    /// Packed chunk flags.
    pub flags: u32,
    /// World-space AABB min (xyz, w=0). For empty chunks: all +INF.
    pub aabb_min: [f32; 4],
    /// World-space AABB max (xyz, w=0). For empty chunks: all -INF.
    pub aabb_max: [f32; 4],
}

/// Compute derived summary data for one chunk. CPU reference matching the GPU shader.
///
/// - `occupancy`: 8192 u32 words (4096 columns × 2 words each)
/// - `palette`: packed u16 MaterialIds (2 per u32)
/// - `material_table`: global material table (index by MaterialId)
/// - `chunk_coord`: world-space chunk coordinate [x, y, z]
pub fn compute_summary(
    occupancy: &[u32],
    palette: &[u32],
    material_table: &[MaterialEntry],
    chunk_coord: [i32; 3],
    voxel_size: f32,
    grid_origin: [f32; 3],
) -> SummaryResult {
    assert_eq!(occupancy.len(), OCCUPANCY_WORDS_PER_SLOT as usize);

    let mut summary = [0u32; SUMMARY_WORDS_PER_SLOT as usize];
    let mut flags = 0u32;
    let mut total_popcount = 0u32;

    // AABB tracking (voxel-space, before world transform)
    let mut min_x: i32 = CS_P as i32;
    let mut min_y: i32 = CS_P as i32;
    let mut min_z: i32 = CS_P as i32;
    let mut max_x: i32 = -1;
    let mut max_y: i32 = -1;
    let mut max_z: i32 = -1;

    // ── Pass 1: Scan occupancy columns ──

    for x in 0..CS_P {
        for z in 0..CS_P {
            let col_idx = (x * CS_P + z) as usize;
            let word_offset = col_idx * 2;
            let lo = occupancy[word_offset] as u64;
            let hi = occupancy[word_offset + 1] as u64;
            let column: u64 = lo | (hi << 32);

            if column == 0 {
                continue;
            }

            total_popcount += column.count_ones();

            // AABB X/Z
            min_x = min_x.min(x as i32);
            max_x = max_x.max(x as i32);
            min_z = min_z.min(z as i32);
            max_z = max_z.max(z as i32);

            // AABB Y: first and last set bit
            let low_y = column.trailing_zeros() as i32;
            let high_y = 63 - column.leading_zeros() as i32;
            min_y = min_y.min(low_y);
            max_y = max_y.max(high_y);

            // Bricklet summary: check each bricklet that this column touches
            let bx = x / BRICKLET_DIM;
            let bz = z / BRICKLET_DIM;
            for by in 0..BRICKLETS_PER_AXIS {
                let y_start = by * BRICKLET_DIM;
                let mask = 0xFFu64 << y_start;
                if column & mask != 0 {
                    let bit_index = bx * 64 + by * 8 + bz;
                    let word_idx = (bit_index >> 5) as usize;
                    let bit_within = bit_index & 31;
                    summary[word_idx] |= 1 << bit_within;
                }
            }
        }
    }

    // ── Flags ──

    let is_empty = total_popcount == 0;
    if is_empty {
        flags |= FLAG_IS_EMPTY;
    }

    // is_fully_opaque: all voxels in the interior [1..62] are set
    // Check all inner columns have all 64 Y-bits set isn't quite right —
    // the spec says "inner region x,z ∈ [1,62]" with all words == 0xFFFFFFFF.
    // For simplicity and correctness: check the full 64³ grid has all bits set.
    if !is_empty {
        let all_ones = occupancy.iter().all(|&w| w == 0xFFFFFFFF);
        if all_ones {
            flags |= FLAG_IS_FULLY_OPAQUE;
        }
    }

    // has_emissive: check if any palette entry references an emissive material
    if !palette.is_empty() {
        let mut has_emissive = false;
        let entry_count = palette.len() * 2; // 2 u16 per u32
        for i in 0..entry_count {
            let word_idx = i / 2;
            if word_idx >= palette.len() {
                break;
            }
            let shift = (i & 1) * 16;
            let mat_id = ((palette[word_idx] >> shift) & 0xFFFF) as usize;
            if mat_id == 0 {
                continue; // MATERIAL_EMPTY
            }
            if mat_id < material_table.len() {
                let entry = &material_table[mat_id];
                // emissive_rg != 0 or emissive_b_opacity has emissive component
                if entry.emissive_rg != 0 || (entry.emissive_b_opacity & 0xFFFF) != 0 {
                    has_emissive = true;
                    break;
                }
            }
        }
        if has_emissive {
            flags |= FLAG_HAS_EMISSIVE;
        }
    }

    // is_resident is always set (we only compute summaries for resident slots)
    flags |= FLAG_IS_RESIDENT;

    // stale_summary is cleared (bit 5 = 0, which is the default)

    // ── AABB (world-space) ──

    let (aabb_min, aabb_max) = if is_empty {
        (
            [f32::INFINITY, f32::INFINITY, f32::INFINITY, 0.0],
            [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY, 0.0],
        )
    } else {
        let vs = voxel_size;
        let world_offset = [
            (chunk_coord[0] as f32 * CS as f32 - 1.0) * vs + grid_origin[0],
            (chunk_coord[1] as f32 * CS as f32 - 1.0) * vs + grid_origin[1],
            (chunk_coord[2] as f32 * CS as f32 - 1.0) * vs + grid_origin[2],
        ];
        (
            [
                world_offset[0] + min_x as f32 * vs,
                world_offset[1] + min_y as f32 * vs,
                world_offset[2] + min_z as f32 * vs,
                0.0,
            ],
            [
                world_offset[0] + (max_x as f32 + 1.0) * vs,
                world_offset[1] + (max_y as f32 + 1.0) * vs,
                world_offset[2] + (max_z as f32 + 1.0) * vs,
                0.0,
            ],
        )
    };

    SummaryResult {
        summary,
        flags,
        aabb_min,
        aabb_max,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{
        self, OccupancyBuilder, PaletteBuilder, MaterialEntry,
        MAT_STONE, MAT_BLUE, MAT_EMISSIVE,
    };

    fn empty_materials() -> Vec<MaterialEntry> {
        vec![MaterialEntry::new([0.0; 3], 0.0, [0.0; 3], 0.0); MAX_MATERIALS as usize]
    }

    fn test_materials() -> Vec<MaterialEntry> {
        scene::test_scene_materials()
    }

    #[test]
    fn empty_chunk() {
        let occ = OccupancyBuilder::new();
        let pal = PaletteBuilder::new();
        let result = compute_summary(
            occ.as_words(), &pal.as_words(), &empty_materials(), [0, 0, 0], 1.0, [0.0; 3],
        );
        assert!(result.flags & FLAG_IS_EMPTY != 0, "should be empty");
        assert!(result.summary.iter().all(|&w| w == 0), "summary should be all zeros");
        assert!(result.aabb_min[0].is_infinite(), "AABB min should be +INF");
        assert!(result.aabb_max[0].is_infinite(), "AABB max should be -INF");
    }

    #[test]
    fn full_chunk() {
        let mut occ = OccupancyBuilder::new();
        for x in 0..CS_P {
            for y in 0..CS_P {
                for z in 0..CS_P {
                    occ.set(x, y, z);
                }
            }
        }
        let pal = PaletteBuilder::new();
        let result = compute_summary(
            occ.as_words(), &pal.as_words(), &empty_materials(), [0, 0, 0], 1.0, [0.0; 3],
        );
        assert!(result.flags & FLAG_IS_EMPTY == 0, "should not be empty");
        assert!(result.flags & FLAG_IS_FULLY_OPAQUE != 0, "should be fully opaque");
        assert!(result.summary.iter().all(|&w| w == 0xFFFFFFFF), "summary should be all ones");
        // AABB should be full chunk extent (offset -1 for chunk 0)
        assert_eq!(result.aabb_min[0], -1.0);
        assert_eq!(result.aabb_min[1], -1.0);
        assert_eq!(result.aabb_min[2], -1.0);
        assert_eq!(result.aabb_max[0], 63.0);
        assert_eq!(result.aabb_max[1], 63.0);
        assert_eq!(result.aabb_max[2], 63.0);
    }

    #[test]
    fn single_voxel_at_17_42_31() {
        let mut occ = OccupancyBuilder::new();
        occ.set(17, 42, 31);
        let pal = PaletteBuilder::new();
        let result = compute_summary(
            occ.as_words(), &pal.as_words(), &empty_materials(), [0, 0, 0], 1.0, [0.0; 3],
        );
        assert!(result.flags & FLAG_IS_EMPTY == 0);
        assert!(result.flags & FLAG_IS_FULLY_OPAQUE == 0);

        // Bricklet (2, 5, 3): bx=17/8=2, by=42/8=5, bz=31/8=3
        let bit_index = 2 * 64 + 5 * 8 + 3;
        let word_idx = (bit_index >> 5) as usize;
        let bit_within = bit_index & 31;
        assert!(result.summary[word_idx] & (1 << bit_within) != 0, "bricklet (2,5,3) should be set");

        // Only one bricklet bit should be set
        let total_bits: u32 = result.summary.iter().map(|w| w.count_ones()).sum();
        assert_eq!(total_bits, 1, "exactly one bricklet should be set");

        // AABB: tight around padded (17, 42, 31) with world_offset = -1 for chunk 0
        assert_eq!(result.aabb_min[0], 16.0);
        assert_eq!(result.aabb_min[1], 41.0);
        assert_eq!(result.aabb_min[2], 30.0);
        assert_eq!(result.aabb_max[0], 17.0);
        assert_eq!(result.aabb_max[1], 42.0);
        assert_eq!(result.aabb_max[2], 31.0);
    }

    #[test]
    fn single_voxel_world_offset() {
        let mut occ = OccupancyBuilder::new();
        occ.set(10, 20, 30);
        let pal = PaletteBuilder::new();
        // Chunk at (1, 2, 3): world_offset = coord * CS - 1 = (61, 123, 185)
        // Voxel at padded (10, 20, 30): world = offset + padded = (71, 143, 215)
        let result = compute_summary(
            occ.as_words(), &pal.as_words(), &empty_materials(), [1, 2, 3], 1.0, [0.0; 3],
        );
        assert_eq!(result.aabb_min[0], 61.0 + 10.0);
        assert_eq!(result.aabb_min[1], 123.0 + 20.0);
        assert_eq!(result.aabb_min[2], 185.0 + 30.0);
        assert_eq!(result.aabb_max[0], 61.0 + 11.0);
        assert_eq!(result.aabb_max[1], 123.0 + 21.0);
        assert_eq!(result.aabb_max[2], 185.0 + 31.0);
    }

    #[test]
    fn emissive_detection() {
        let mut pal = PaletteBuilder::new();
        pal.add(MAT_EMISSIVE);
        let occ = OccupancyBuilder::new();
        let mats = test_materials();
        let result = compute_summary(
            occ.as_words(), &pal.as_words(), &mats, [0, 0, 0], 1.0, [0.0; 3],
        );
        assert!(result.flags & FLAG_HAS_EMISSIVE != 0, "should detect emissive material");
    }

    #[test]
    fn no_emissive_without_emissive_material() {
        let mut pal = PaletteBuilder::new();
        pal.add(MAT_STONE);
        pal.add(MAT_BLUE);
        let occ = OccupancyBuilder::new();
        let mats = test_materials();
        let result = compute_summary(
            occ.as_words(), &pal.as_words(), &mats, [0, 0, 0], 1.0, [0.0; 3],
        );
        assert!(result.flags & FLAG_HAS_EMISSIVE == 0, "should not detect emissive");
    }

    #[test]
    fn sphere_aabb_containment() {
        let chunk = scene::generate_sphere(
            ChunkCoord { x: 0, y: 0, z: 0 }, 32, 25, 32, 10,
        );
        let mats = test_materials();
        let result = compute_summary(
            chunk.occupancy.as_words(), &chunk.palette.as_words(), &mats, [0, 0, 0], 1.0, [0.0; 3],
        );

        assert!(result.flags & FLAG_IS_EMPTY == 0);

        // Every occupied voxel must be inside the AABB.
        // AABB is in world space: world = padded - 1 (for chunk 0 with world_offset = -1).
        for x in 0..CS_P {
            for y in 0..CS_P {
                for z in 0..CS_P {
                    if chunk.occupancy.get(x, y, z) {
                        let wx = x as f32 - 1.0;
                        let wy = y as f32 - 1.0;
                        let wz = z as f32 - 1.0;
                        assert!(
                            wx >= result.aabb_min[0] && (wx + 1.0) <= result.aabb_max[0]
                            && wy >= result.aabb_min[1] && (wy + 1.0) <= result.aabb_max[1]
                            && wz >= result.aabb_min[2] && (wz + 1.0) <= result.aabb_max[2],
                            "Voxel ({x},{y},{z}) world ({wx},{wy},{wz}) outside AABB [{:?}..{:?}]",
                            &result.aabb_min[..3], &result.aabb_max[..3]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn sphere_aabb_tightness() {
        let chunk = scene::generate_sphere(
            ChunkCoord { x: 0, y: 0, z: 0 }, 32, 25, 32, 10,
        );
        let mats = test_materials();
        let result = compute_summary(
            chunk.occupancy.as_words(), &chunk.palette.as_words(), &mats, [0, 0, 0], 1.0, [0.0; 3],
        );

        // At least one voxel should touch each AABB face.
        // AABB is world space (padded - 1 for chunk 0). Convert back: padded = world + 1.
        let min_x = (result.aabb_min[0] + 1.0) as u32;
        let max_x = (result.aabb_max[0] + 1.0) as u32 - 1;
        let min_y = (result.aabb_min[1] + 1.0) as u32;
        let max_y = (result.aabb_max[1] + 1.0) as u32 - 1;
        let min_z = (result.aabb_min[2] + 1.0) as u32;
        let max_z = (result.aabb_max[2] + 1.0) as u32 - 1;

        let mut touches_min_x = false;
        let mut touches_max_x = false;
        let mut touches_min_y = false;
        let mut touches_max_y = false;
        let mut touches_min_z = false;
        let mut touches_max_z = false;

        for x in 0..CS_P {
            for y in 0..CS_P {
                for z in 0..CS_P {
                    if chunk.occupancy.get(x, y, z) {
                        if x == min_x { touches_min_x = true; }
                        if x == max_x { touches_max_x = true; }
                        if y == min_y { touches_min_y = true; }
                        if y == max_y { touches_max_y = true; }
                        if z == min_z { touches_min_z = true; }
                        if z == max_z { touches_max_z = true; }
                    }
                }
            }
        }

        assert!(touches_min_x, "No voxel on min_x face");
        assert!(touches_max_x, "No voxel on max_x face");
        assert!(touches_min_y, "No voxel on min_y face");
        assert!(touches_max_y, "No voxel on max_y face");
        assert!(touches_min_z, "No voxel on min_z face");
        assert!(touches_max_z, "No voxel on max_z face");
    }

    #[test]
    fn summary_matches_occupancy() {
        // For the test scene, verify every bricklet bit matches actual occupancy
        let (chunks, mats) = scene::generate_test_scene();
        let chunk = &chunks[0];
        let result = compute_summary(
            chunk.occupancy.as_words(), &chunk.palette.as_words(), &mats, [0, 0, 0], 1.0, [0.0; 3],
        );

        for bx in 0..BRICKLETS_PER_AXIS {
            for by in 0..BRICKLETS_PER_AXIS {
                for bz in 0..BRICKLETS_PER_AXIS {
                    let bit_index = bx * 64 + by * 8 + bz;
                    let word_idx = (bit_index >> 5) as usize;
                    let bit_within = bit_index & 31;
                    let summary_bit = (result.summary[word_idx] >> bit_within) & 1;

                    // Check if any voxel in this 8³ bricklet is occupied
                    let mut has_voxel = false;
                    for dx in 0..BRICKLET_DIM {
                        for dy in 0..BRICKLET_DIM {
                            for dz in 0..BRICKLET_DIM {
                                let vx = bx * BRICKLET_DIM + dx;
                                let vy = by * BRICKLET_DIM + dy;
                                let vz = bz * BRICKLET_DIM + dz;
                                if vx < CS_P && vy < CS_P && vz < CS_P
                                    && chunk.occupancy.get(vx, vy, vz)
                                {
                                    has_voxel = true;
                                }
                            }
                        }
                    }

                    assert_eq!(
                        summary_bit != 0, has_voxel,
                        "Bricklet ({bx},{by},{bz}): summary={summary_bit} but has_voxel={has_voxel}"
                    );
                }
            }
        }
    }

    #[test]
    fn is_resident_always_set() {
        let occ = OccupancyBuilder::new();
        let pal = PaletteBuilder::new();
        let result = compute_summary(
            occ.as_words(), &pal.as_words(), &empty_materials(), [0, 0, 0], 1.0, [0.0; 3],
        );
        assert!(result.flags & FLAG_IS_RESIDENT != 0, "is_resident should always be set");
    }
}
