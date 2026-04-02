//! Input conversion functions to BinaryChunk format.
//!
//! Provides conversion from various input formats:
//! - Position arrays (voxel center coordinates)
//! - Dense voxel arrays (material ID per voxel)

use crate::core::{BinaryChunk, MaterialId, MATERIAL_EMPTY, CS_P, CS};

/// Epsilon for robust float-to-int conversion.
/// Handles floating point edge cases near integer boundaries.
const COORD_EPSILON: f32 = 1e-5;

/// Robust floor that handles values very close to integers.
///
/// When a value is within COORD_EPSILON of an integer, rounds to that integer
/// instead of flooring. This prevents off-by-one errors from floating point
/// precision issues.
#[inline]
fn robust_floor(value: f32) -> i32 {
    let rounded = value.round();
    if (value - rounded).abs() < COORD_EPSILON {
        rounded as i32
    } else {
        value.floor() as i32
    }
}

/// Convert voxel center positions to binary chunk.
///
/// Positions are world-space (x, y, z) tuples packed as a flat array.
/// Voxels outside chunk bounds are ignored.
///
/// # Arguments
/// * `positions` - Flat array of voxel positions (x, y, z triples)
/// * `voxel_size` - Size of each voxel in world units
/// * `chunk_origin` - World position of chunk origin
/// * `material` - Material ID to assign to all voxels
///
/// # Example
/// ```
/// use greedy_mesher::positions_to_binary_chunk;
///
/// let positions = [0.5, 0.5, 0.5, 1.5, 0.5, 0.5]; // Two voxels
/// let chunk = positions_to_binary_chunk(&positions, 1.0, [0.0, 0.0, 0.0], 1);
///
/// assert!(chunk.is_solid(1, 1, 1)); // First voxel (with +1 padding offset)
/// assert!(chunk.is_solid(2, 1, 1)); // Second voxel
/// ```
pub fn positions_to_binary_chunk(
    positions: &[f32],
    voxel_size: f32,
    chunk_origin: [f32; 3],
    material: MaterialId,
) -> BinaryChunk {
    let mut chunk = BinaryChunk::new();
    let inv_size = 1.0 / voxel_size;

    for pos in positions.chunks_exact(3) {
        // Convert world position to chunk-local voxel coordinates
        // Use robust_floor to handle floating point edge cases
        let lx = robust_floor((pos[0] - chunk_origin[0]) * inv_size) + 1;
        let ly = robust_floor((pos[1] - chunk_origin[1]) * inv_size) + 1;
        let lz = robust_floor((pos[2] - chunk_origin[2]) * inv_size) + 1;

        // Check bounds (usable range is 1 to CS_P-2 inclusive due to padding)
        if lx >= 1 && lx < (CS_P - 1) as i32
            && ly >= 1 && ly < (CS_P - 1) as i32
            && lz >= 1 && lz < (CS_P - 1) as i32
        {
            chunk.set(lx as usize, ly as usize, lz as usize, material);
        }
    }

    chunk
}

/// Convert dense voxel array to binary chunk.
///
/// Input is material ID per voxel (0 = empty), stored in X-major order:
/// `voxels[x + y * width + z * width * height]`
///
/// WARNING: This allocates ~544KB on the stack. For WASM or stack-constrained
/// environments, use `dense_to_binary_chunk_boxed()`.
///
/// # Arguments
/// * `voxels` - Material ID per voxel (0 = empty)
/// * `dims` - Dimensions [width, height, depth]
///
/// # Example
/// ```
/// use greedy_mesher::dense_to_binary_chunk;
///
/// let mut voxels = vec![0u16; 4 * 4 * 4];
/// voxels[0] = 1; // Set voxel at (0, 0, 0)
///
/// let chunk = dense_to_binary_chunk(&voxels, [4, 4, 4]);
/// assert!(chunk.is_solid(1, 1, 1)); // +1 offset for padding
/// ```
pub fn dense_to_binary_chunk(
    voxels: &[MaterialId],
    dims: [usize; 3],
) -> BinaryChunk {
    let mut chunk = BinaryChunk::new();
    dense_fill_chunk(&mut chunk, voxels, dims);
    chunk
}

/// Convert dense voxel array to binary chunk (heap-allocated).
///
/// This is the preferred method for WASM and stack-constrained environments.
/// Uses `BinaryChunk::new_boxed()` to avoid stack overflow.
///
/// # Arguments
/// * `voxels` - Material ID per voxel (0 = empty)
/// * `dims` - Dimensions [width, height, depth]
pub fn dense_to_binary_chunk_boxed(
    voxels: &[MaterialId],
    dims: [usize; 3],
) -> Box<BinaryChunk> {
    let mut chunk = BinaryChunk::new_boxed();
    dense_fill_chunk(&mut chunk, voxels, dims);
    chunk
}

/// Fill a chunk from dense voxel data (shared implementation).
fn dense_fill_chunk(
    chunk: &mut BinaryChunk,
    voxels: &[MaterialId],
    dims: [usize; 3],
) {
    let [dx, dy, dz] = dims;

    for z in 0..dz.min(CS) {
        for y in 0..dy.min(CS) {
            for x in 0..dx.min(CS) {
                let src_idx = x + y * dx + z * dx * dy;
                if src_idx < voxels.len() {
                    let material = voxels[src_idx];
                    if material != MATERIAL_EMPTY {
                        // +1 offset for padding
                        chunk.set(x + 1, y + 1, z + 1, material);
                    }
                }
            }
        }
    }
}

/// Convert dense voxel array with per-voxel materials to binary chunk.
///
/// Alternative indexing: Z-major order (common in some voxel formats).
/// `voxels[z + y * depth + x * depth * height]`
pub fn dense_to_binary_chunk_zyx(
    voxels: &[MaterialId],
    dims: [usize; 3],
) -> BinaryChunk {
    let mut chunk = BinaryChunk::new();
    dense_fill_chunk_zyx(&mut chunk, voxels, dims);
    chunk
}

/// Convert dense voxel array to binary chunk (heap-allocated, Z-major order).
pub fn dense_to_binary_chunk_zyx_boxed(
    voxels: &[MaterialId],
    dims: [usize; 3],
) -> Box<BinaryChunk> {
    let mut chunk = BinaryChunk::new_boxed();
    dense_fill_chunk_zyx(&mut chunk, voxels, dims);
    chunk
}

/// Fill a chunk from dense ZYX voxel data (shared implementation).
fn dense_fill_chunk_zyx(
    chunk: &mut BinaryChunk,
    voxels: &[MaterialId],
    dims: [usize; 3],
) {
    let [dx, dy, dz] = dims;

    for x in 0..dx.min(CS) {
        for y in 0..dy.min(CS) {
            for z in 0..dz.min(CS) {
                let src_idx = z + y * dz + x * dz * dy;
                if src_idx < voxels.len() {
                    let material = voxels[src_idx];
                    if material != MATERIAL_EMPTY {
                        chunk.set(x + 1, y + 1, z + 1, material);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robust_floor_normal() {
        assert_eq!(robust_floor(1.5), 1);
        assert_eq!(robust_floor(1.9), 1);
        assert_eq!(robust_floor(2.1), 2);
    }

    #[test]
    fn robust_floor_near_integer() {
        // Values very close to integers should round to that integer
        assert_eq!(robust_floor(1.999999), 2);
        assert_eq!(robust_floor(2.000001), 2);
        assert_eq!(robust_floor(0.999999), 1);
    }

    #[test]
    fn robust_floor_negative() {
        assert_eq!(robust_floor(-0.5), -1);
        assert_eq!(robust_floor(-1.5), -2);
        assert_eq!(robust_floor(-0.000001), 0); // Near zero
    }

    #[test]
    fn positions_single_voxel() {
        let positions = [0.5, 0.5, 0.5]; // Center of voxel (0,0,0)
        let chunk = positions_to_binary_chunk(&positions, 1.0, [0.0, 0.0, 0.0], 1);

        // With +1 padding offset, voxel at (0,0,0) maps to (1,1,1)
        assert!(chunk.is_solid(1, 1, 1));
        assert_eq!(chunk.get_material(1, 1, 1), 1);
        assert_eq!(chunk.solid_count(), 1);
    }

    #[test]
    fn positions_multiple_voxels() {
        let positions = [
            0.5, 0.5, 0.5,
            1.5, 0.5, 0.5,
            2.5, 0.5, 0.5,
        ];
        let chunk = positions_to_binary_chunk(&positions, 1.0, [0.0, 0.0, 0.0], 42);

        assert!(chunk.is_solid(1, 1, 1));
        assert!(chunk.is_solid(2, 1, 1));
        assert!(chunk.is_solid(3, 1, 1));
        assert_eq!(chunk.solid_count(), 3);
    }

    #[test]
    fn positions_with_origin_offset() {
        let positions = [10.5, 20.5, 30.5];
        let chunk = positions_to_binary_chunk(&positions, 1.0, [10.0, 20.0, 30.0], 1);

        assert!(chunk.is_solid(1, 1, 1));
        assert_eq!(chunk.solid_count(), 1);
    }

    #[test]
    fn positions_with_voxel_size() {
        let positions = [0.05, 0.05, 0.05]; // Voxel (0,0,0) with size 0.1
        let chunk = positions_to_binary_chunk(&positions, 0.1, [0.0, 0.0, 0.0], 1);

        assert!(chunk.is_solid(1, 1, 1));
    }

    #[test]
    fn positions_out_of_bounds_ignored() {
        let positions = [
            0.5, 0.5, 0.5,      // Valid
            100.0, 0.5, 0.5,    // Out of bounds (x too large)
            -100.0, 0.5, 0.5,   // Out of bounds (x negative)
        ];
        let chunk = positions_to_binary_chunk(&positions, 1.0, [0.0, 0.0, 0.0], 1);

        assert_eq!(chunk.solid_count(), 1); // Only one valid voxel
    }

    #[test]
    fn dense_single_voxel() {
        let mut voxels = vec![0u16; 4 * 4 * 4];
        voxels[0] = 1; // (0, 0, 0)

        let chunk = dense_to_binary_chunk(&voxels, [4, 4, 4]);

        assert!(chunk.is_solid(1, 1, 1)); // +1 padding
        assert_eq!(chunk.solid_count(), 1);
    }

    #[test]
    fn dense_multiple_voxels() {
        let mut voxels = vec![0u16; 4 * 4 * 4];
        // Set voxels at (0,0,0), (1,0,0), (0,1,0)
        voxels[0] = 1;
        voxels[1] = 2;
        voxels[4] = 3;

        let chunk = dense_to_binary_chunk(&voxels, [4, 4, 4]);

        assert!(chunk.is_solid(1, 1, 1));
        assert_eq!(chunk.get_material(1, 1, 1), 1);

        assert!(chunk.is_solid(2, 1, 1));
        assert_eq!(chunk.get_material(2, 1, 1), 2);

        assert!(chunk.is_solid(1, 2, 1));
        assert_eq!(chunk.get_material(1, 2, 1), 3);

        assert_eq!(chunk.solid_count(), 3);
    }

    #[test]
    fn dense_full_chunk() {
        // Fill a small chunk completely
        let voxels = vec![1u16; 8 * 8 * 8];
        let chunk = dense_to_binary_chunk(&voxels, [8, 8, 8]);

        assert_eq!(chunk.solid_count(), 8 * 8 * 8);
    }

    #[test]
    fn dense_oversized_input() {
        // Input larger than chunk - should clamp to CS
        let voxels = vec![1u16; 100 * 100 * 100];
        let chunk = dense_to_binary_chunk(&voxels, [100, 100, 100]);

        // Should only have CS^3 voxels
        assert_eq!(chunk.solid_count(), CS * CS * CS);
    }
}
