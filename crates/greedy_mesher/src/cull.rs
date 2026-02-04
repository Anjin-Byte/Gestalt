//! Bitwise face culling.
//!
//! Uses bitwise operations to determine which voxel faces are visible.
//! A face is visible if the voxel is solid AND the neighbor in that direction is empty.
//!
//! This processes 64 voxels per bitwise operation, achieving significant speedup
//! over per-voxel iteration.

use crate::core::{
    BinaryChunk, FaceMasks, CS_P, CS,
    FACE_POS_Y, FACE_NEG_Y, FACE_POS_X, FACE_NEG_X, FACE_POS_Z, FACE_NEG_Z,
};

/// Generate face masks using bitwise neighbor culling.
///
/// A face is visible if:
/// 1. The voxel is solid (bit set in opaque_mask)
/// 2. The neighbor in that direction is empty (bit not set)
///
/// For Y-axis neighbors, we use bit shifts within the column.
/// For X/Z-axis neighbors, we compare with adjacent columns.
///
/// The resulting masks indicate which voxels have visible faces in each direction.
/// Coordinates are shifted by 1 to account for padding.
pub fn cull_faces(chunk: &BinaryChunk, masks: &mut FaceMasks) {
    // Mask for usable voxels (bits 1-62, shifted to 0-61 after >> 1)
    let usable_mask: u64 = (1u64 << CS) - 1;

    for x in 1..(CS_P - 1) {
        let x_cs_p = x * CS_P;

        for z in 1..(CS_P - 1) {
            let column_idx = x_cs_p + z;
            let column = chunk.opaque_mask[column_idx];

            // Skip empty columns
            if column == 0 {
                continue;
            }

            // +Y face: visible where solid AND y+1 is empty
            // column >> 1 shifts to check y+1 neighbor
            // ~(column >> 1) gives us positions where y+1 is empty
            // column & ~(column >> 1) gives us solid voxels with empty above
            let pos_y = column & !(column >> 1);
            // Shift right by 1 to convert from padded to usable coordinates
            masks.set(FACE_POS_Y, x, z, (pos_y >> 1) & usable_mask);

            // -Y face: visible where solid AND y-1 is empty
            // column << 1 shifts to check y-1 neighbor
            let neg_y = column & !(column << 1);
            masks.set(FACE_NEG_Y, x, z, (neg_y >> 1) & usable_mask);

            // +X face: compare with x+1 column
            let neighbor_pos_x = chunk.opaque_mask[(x + 1) * CS_P + z];
            let pos_x = column & !neighbor_pos_x;
            masks.set(FACE_POS_X, x, z, (pos_x >> 1) & usable_mask);

            // -X face: compare with x-1 column
            let neighbor_neg_x = chunk.opaque_mask[(x - 1) * CS_P + z];
            let neg_x = column & !neighbor_neg_x;
            masks.set(FACE_NEG_X, x, z, (neg_x >> 1) & usable_mask);

            // +Z face: compare with z+1 column
            let neighbor_pos_z = chunk.opaque_mask[x_cs_p + z + 1];
            let pos_z = column & !neighbor_pos_z;
            masks.set(FACE_POS_Z, x, z, (pos_z >> 1) & usable_mask);

            // -Z face: compare with z-1 column
            let neighbor_neg_z = chunk.opaque_mask[x_cs_p + z - 1];
            let neg_z = column & !neighbor_neg_z;
            masks.set(FACE_NEG_Z, x, z, (neg_z >> 1) & usable_mask);
        }
    }
}

/// Count total visible faces after culling.
/// Useful for statistics and debugging.
pub fn count_visible_faces(masks: &FaceMasks) -> [usize; 6] {
    let mut counts = [0usize; 6];
    for face in 0..6 {
        for x in 1..(CS_P - 1) {
            for z in 1..(CS_P - 1) {
                counts[face] += masks.get(face, x, z).count_ones() as usize;
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_voxel_six_faces() {
        let mut chunk = BinaryChunk::new();
        // Place voxel at (32, 32, 32) - center of chunk
        chunk.set(32, 32, 32, 1);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        // Count total faces
        let total: usize = masks.total_faces();
        assert_eq!(total, 6, "Single voxel should have 6 visible faces");
    }

    #[test]
    fn two_adjacent_y_share_face() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);
        chunk.set(32, 33, 32, 1); // Adjacent in +Y

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        // Two voxels: 12 faces - 2 hidden = 10
        let total = masks.total_faces();
        assert_eq!(total, 10);
    }

    #[test]
    fn two_adjacent_x_share_face() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);
        chunk.set(33, 32, 32, 1); // Adjacent in +X

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        // Two voxels: 12 faces - 2 hidden = 10
        let total = masks.total_faces();
        assert_eq!(total, 10);
    }

    #[test]
    fn two_adjacent_z_share_face() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);
        chunk.set(32, 32, 33, 1); // Adjacent in +Z

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        // Two voxels: 12 faces - 2 hidden = 10
        let total = masks.total_faces();
        assert_eq!(total, 10);
    }

    #[test]
    fn cube_3x3x3_interior_hidden() {
        let mut chunk = BinaryChunk::new();
        // 3x3x3 cube centered at (32, 32, 32)
        for x in 31..34 {
            for y in 31..34 {
                for z in 31..34 {
                    chunk.set(x, y, z, 1);
                }
            }
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        // Center voxel (32, 32, 32) should have 0 visible faces
        // Surface of 3x3x3 cube has 9 faces per side = 54 total
        let total = masks.total_faces();
        assert_eq!(total, 54, "3x3x3 cube should have 54 visible faces (9 per side)");
    }

    #[test]
    fn empty_chunk_no_faces() {
        let chunk = BinaryChunk::new();

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        assert_eq!(masks.total_faces(), 0);
    }

    #[test]
    fn boundary_voxel_at_x1() {
        let mut chunk = BinaryChunk::new();
        // Voxel at x=1 (boundary with padding)
        chunk.set(1, 32, 32, 1);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        // Should have 6 faces (all exposed since it's at boundary)
        assert_eq!(masks.total_faces(), 6);

        // Should have -X face visible at the boundary
        let neg_x_mask = masks.get(FACE_NEG_X, 1, 32);
        assert!(neg_x_mask != 0, "-X face should be visible at x=1 boundary");
    }

    #[test]
    fn face_direction_counts() {
        let mut chunk = BinaryChunk::new();
        // Solid slab on Y=32 from x=20-30, z=20-30
        for x in 20..30 {
            for z in 20..30 {
                chunk.set(x, 32, z, 1);
            }
        }

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        let counts = count_visible_faces(&masks);

        // +Y and -Y faces: 10*10 = 100 each
        assert_eq!(counts[FACE_POS_Y], 100);
        assert_eq!(counts[FACE_NEG_Y], 100);

        // +X/-X edges: 10 voxels each
        assert_eq!(counts[FACE_POS_X], 10);
        assert_eq!(counts[FACE_NEG_X], 10);

        // +Z/-Z edges: 10 voxels each
        assert_eq!(counts[FACE_POS_Z], 10);
        assert_eq!(counts[FACE_NEG_Z], 10);
    }

    #[test]
    fn column_at_different_y_levels() {
        let mut chunk = BinaryChunk::new();
        // Voxels at different Y levels in same column
        chunk.set(32, 10, 32, 1);
        chunk.set(32, 30, 32, 1);
        chunk.set(32, 50, 32, 1);

        let mut masks = FaceMasks::new();
        cull_faces(&chunk, &mut masks);

        // 3 separate voxels = 18 faces
        assert_eq!(masks.total_faces(), 18);
    }
}
