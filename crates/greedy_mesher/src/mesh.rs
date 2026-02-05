//! Main meshing pipeline.
//!
//! Orchestrates the complete meshing process:
//! 1. Face culling (determine visible faces)
//! 2. Greedy merge (combine adjacent faces into quads)
//! 3. Quad expansion (convert quads to vertex arrays)

use crate::core::{
    BinaryChunk, FaceMasks, MeshOutput,
    FACE_POS_Y, FACE_NEG_Y, FACE_POS_X, FACE_NEG_X, FACE_POS_Z, FACE_NEG_Z,
};
use crate::cull::cull_faces;
use crate::merge::{greedy_merge_y_faces, greedy_merge_x_faces, greedy_merge_z_faces};
use crate::expand::{expand_quads, expand_quads_with_uvs};

/// Mesh a binary chunk into geometry (positions, normals, indices).
///
/// This is the main entry point for meshing. It performs:
/// 1. Face culling using bitwise operations
/// 2. Greedy merge to combine adjacent faces
/// 3. Quad expansion to vertex arrays
///
/// # Arguments
/// * `chunk` - The binary chunk to mesh
/// * `voxel_size` - Size of each voxel in world units
/// * `origin` - World position offset for the chunk
///
/// # Example
/// ```
/// use greedy_mesher::{BinaryChunk, mesh_chunk};
///
/// let mut chunk = BinaryChunk::new();
/// chunk.set(32, 32, 32, 1);
///
/// let mesh = mesh_chunk(&chunk, 1.0, [0.0, 0.0, 0.0]);
/// assert!(mesh.triangle_count() > 0);
/// ```
pub fn mesh_chunk(chunk: &BinaryChunk, voxel_size: f32, origin: [f32; 3]) -> MeshOutput {
    // Early exit for empty chunks
    if chunk.is_empty() {
        return MeshOutput::default();
    }

    // Step 1: Face culling
    let mut masks = FaceMasks::new();
    cull_faces(chunk, &mut masks);

    // Step 2: Greedy merge for each face direction
    let mut packed_quads: [Vec<u64>; 6] = Default::default();

    greedy_merge_y_faces(FACE_POS_Y, chunk, &masks, &mut packed_quads[FACE_POS_Y]);
    greedy_merge_y_faces(FACE_NEG_Y, chunk, &masks, &mut packed_quads[FACE_NEG_Y]);
    greedy_merge_x_faces(FACE_POS_X, chunk, &masks, &mut packed_quads[FACE_POS_X]);
    greedy_merge_x_faces(FACE_NEG_X, chunk, &masks, &mut packed_quads[FACE_NEG_X]);
    greedy_merge_z_faces(FACE_POS_Z, chunk, &masks, &mut packed_quads[FACE_POS_Z]);
    greedy_merge_z_faces(FACE_NEG_Z, chunk, &masks, &mut packed_quads[FACE_NEG_Z]);

    // Step 3: Expand quads to vertex arrays
    expand_quads(&packed_quads, voxel_size, origin)
}

/// Mesh with UV coordinates and per-vertex material IDs.
///
/// Same as `mesh_chunk` but includes:
/// - UV coordinates for texture mapping (tiled based on quad size)
/// - Per-vertex material IDs for texture atlas lookup
///
/// Use this when you need textured rendering with a material atlas.
///
/// # Arguments
/// * `chunk` - The binary chunk to mesh
/// * `voxel_size` - Size of each voxel in world units
/// * `origin` - World position offset for the chunk
pub fn mesh_chunk_with_uvs(chunk: &BinaryChunk, voxel_size: f32, origin: [f32; 3]) -> MeshOutput {
    // Early exit for empty chunks
    if chunk.is_empty() {
        return MeshOutput::default();
    }

    // Step 1: Face culling
    let mut masks = FaceMasks::new();
    cull_faces(chunk, &mut masks);

    // Step 2: Greedy merge for each face direction
    let mut packed_quads: [Vec<u64>; 6] = Default::default();

    greedy_merge_y_faces(FACE_POS_Y, chunk, &masks, &mut packed_quads[FACE_POS_Y]);
    greedy_merge_y_faces(FACE_NEG_Y, chunk, &masks, &mut packed_quads[FACE_NEG_Y]);
    greedy_merge_x_faces(FACE_POS_X, chunk, &masks, &mut packed_quads[FACE_POS_X]);
    greedy_merge_x_faces(FACE_NEG_X, chunk, &masks, &mut packed_quads[FACE_NEG_X]);
    greedy_merge_z_faces(FACE_POS_Z, chunk, &masks, &mut packed_quads[FACE_POS_Z]);
    greedy_merge_z_faces(FACE_NEG_Z, chunk, &masks, &mut packed_quads[FACE_NEG_Z]);

    // Step 3: Expand quads with UVs and material IDs
    expand_quads_with_uvs(&packed_quads, voxel_size, origin)
}

/// Statistics about a mesh result.
#[derive(Debug, Clone, Default)]
pub struct MeshStats {
    /// Total number of quads generated
    pub quad_count: usize,
    /// Quads per face direction
    pub quads_per_face: [usize; 6],
    /// Total vertices
    pub vertex_count: usize,
    /// Total triangles
    pub triangle_count: usize,
    /// Theoretical maximum quads (without merging)
    pub max_possible_quads: usize,
    /// Merge efficiency (1.0 = perfect merging, 0.0 = no merging)
    pub merge_efficiency: f32,
}

/// Mesh a chunk and return statistics along with the mesh.
pub fn mesh_chunk_with_stats(
    chunk: &BinaryChunk,
    voxel_size: f32,
    origin: [f32; 3],
) -> (MeshOutput, MeshStats) {
    if chunk.is_empty() {
        return (MeshOutput::default(), MeshStats::default());
    }

    // Face culling
    let mut masks = FaceMasks::new();
    cull_faces(chunk, &mut masks);

    // Count visible faces before merging
    let max_possible_quads = masks.total_faces();

    // Greedy merge
    let mut packed_quads: [Vec<u64>; 6] = Default::default();

    greedy_merge_y_faces(FACE_POS_Y, chunk, &masks, &mut packed_quads[FACE_POS_Y]);
    greedy_merge_y_faces(FACE_NEG_Y, chunk, &masks, &mut packed_quads[FACE_NEG_Y]);
    greedy_merge_x_faces(FACE_POS_X, chunk, &masks, &mut packed_quads[FACE_POS_X]);
    greedy_merge_x_faces(FACE_NEG_X, chunk, &masks, &mut packed_quads[FACE_NEG_X]);
    greedy_merge_z_faces(FACE_POS_Z, chunk, &masks, &mut packed_quads[FACE_POS_Z]);
    greedy_merge_z_faces(FACE_NEG_Z, chunk, &masks, &mut packed_quads[FACE_NEG_Z]);

    // Calculate statistics
    let quads_per_face = [
        packed_quads[FACE_POS_Y].len(),
        packed_quads[FACE_NEG_Y].len(),
        packed_quads[FACE_POS_X].len(),
        packed_quads[FACE_NEG_X].len(),
        packed_quads[FACE_POS_Z].len(),
        packed_quads[FACE_NEG_Z].len(),
    ];
    let quad_count: usize = quads_per_face.iter().sum();

    // Expand to mesh
    let mesh = expand_quads(&packed_quads, voxel_size, origin);

    let merge_efficiency = if max_possible_quads > 0 {
        1.0 - (quad_count as f32 / max_possible_quads as f32)
    } else {
        0.0
    };

    let stats = MeshStats {
        quad_count,
        quads_per_face,
        vertex_count: mesh.vertex_count(),
        triangle_count: mesh.triangle_count(),
        max_possible_quads,
        merge_efficiency,
    };

    (mesh, stats)
}

/// Debug output from meshing pipeline.
pub struct MeshDebugOutput {
    /// The mesh geometry
    pub mesh: MeshOutput,
    /// Mesh statistics
    pub stats: MeshStats,
    /// Debug geometry (wireframes, colors)
    pub debug: crate::debug::DebugGeometry,
    /// Per-direction face statistics
    pub direction_stats: crate::debug::FaceDirectionStats,
}

/// Mesh a chunk and return full debug output.
///
/// This is the primary entry point for debug visualization.
/// Returns the mesh, statistics, wireframe lines, and per-vertex colors.
pub fn mesh_chunk_debug(
    chunk: &BinaryChunk,
    voxel_size: f32,
    origin: [f32; 3],
) -> MeshDebugOutput {
    if chunk.is_empty() {
        return MeshDebugOutput {
            mesh: MeshOutput::default(),
            stats: MeshStats::default(),
            debug: crate::debug::DebugGeometry {
                line_positions: Vec::new(),
                face_colors: Vec::new(),
                size_colors: Vec::new(),
            },
            direction_stats: crate::debug::FaceDirectionStats::default(),
        };
    }

    // Face culling
    let mut masks = FaceMasks::new();
    cull_faces(chunk, &mut masks);
    let max_possible_quads = masks.total_faces();

    // Greedy merge
    let mut packed_quads: [Vec<u64>; 6] = Default::default();
    greedy_merge_y_faces(FACE_POS_Y, chunk, &masks, &mut packed_quads[FACE_POS_Y]);
    greedy_merge_y_faces(FACE_NEG_Y, chunk, &masks, &mut packed_quads[FACE_NEG_Y]);
    greedy_merge_x_faces(FACE_POS_X, chunk, &masks, &mut packed_quads[FACE_POS_X]);
    greedy_merge_x_faces(FACE_NEG_X, chunk, &masks, &mut packed_quads[FACE_NEG_X]);
    greedy_merge_z_faces(FACE_POS_Z, chunk, &masks, &mut packed_quads[FACE_POS_Z]);
    greedy_merge_z_faces(FACE_NEG_Z, chunk, &masks, &mut packed_quads[FACE_NEG_Z]);

    // Stats
    let quads_per_face = [
        packed_quads[FACE_POS_Y].len(),
        packed_quads[FACE_NEG_Y].len(),
        packed_quads[FACE_POS_X].len(),
        packed_quads[FACE_NEG_X].len(),
        packed_quads[FACE_POS_Z].len(),
        packed_quads[FACE_NEG_Z].len(),
    ];
    let quad_count: usize = quads_per_face.iter().sum();

    // Expand mesh
    let mesh = expand_quads(&packed_quads, voxel_size, origin);

    let merge_efficiency = if max_possible_quads > 0 {
        1.0 - (quad_count as f32 / max_possible_quads as f32)
    } else {
        0.0
    };

    let stats = MeshStats {
        quad_count,
        quads_per_face,
        vertex_count: mesh.vertex_count(),
        triangle_count: mesh.triangle_count(),
        max_possible_quads,
        merge_efficiency,
    };

    // Debug geometry
    let debug = crate::debug::generate_debug_geometry(&packed_quads, voxel_size, origin);
    let direction_stats = crate::debug::compute_direction_stats(&packed_quads, max_possible_quads);

    MeshDebugOutput {
        mesh,
        stats,
        debug,
        direction_stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_single_voxel() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 1);

        let mesh = mesh_chunk(&chunk, 1.0, [0.0, 0.0, 0.0]);

        // Single voxel = 6 faces = 6 quads = 24 vertices, 12 triangles
        assert_eq!(mesh.vertex_count(), 24);
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn mesh_empty_chunk() {
        let chunk = BinaryChunk::new();
        let mesh = mesh_chunk(&chunk, 1.0, [0.0, 0.0, 0.0]);

        assert_eq!(mesh.vertex_count(), 0);
        assert_eq!(mesh.triangle_count(), 0);
    }

    #[test]
    fn mesh_with_uvs_has_material_ids() {
        let mut chunk = BinaryChunk::new();
        chunk.set(32, 32, 32, 42);

        let mesh = mesh_chunk_with_uvs(&chunk, 1.0, [0.0, 0.0, 0.0]);

        assert!(mesh.has_uvs());
        assert!(mesh.has_material_ids());
        assert_eq!(mesh.material_ids.len(), mesh.vertex_count());

        // All material IDs should be 42
        for &mat in &mesh.material_ids {
            assert_eq!(mat, 42);
        }
    }

    #[test]
    fn mesh_stats_merge_efficiency() {
        let mut chunk = BinaryChunk::new();
        // 10x10 slab - should merge well
        for x in 20..30 {
            for z in 20..30 {
                chunk.set(x, 32, z, 1);
            }
        }

        let (mesh, stats) = mesh_chunk_with_stats(&chunk, 1.0, [0.0, 0.0, 0.0]);

        // Should have very high merge efficiency for a solid slab
        assert!(stats.merge_efficiency > 0.9, "Expected high merge efficiency, got {}", stats.merge_efficiency);

        // +Y and -Y faces should merge to 1 quad each, edges to few quads
        assert!(stats.quad_count < 50, "Expected few quads from merged slab, got {}", stats.quad_count);
    }

    #[test]
    fn mesh_checkerboard_low_efficiency() {
        let mut chunk = BinaryChunk::new();
        // Checkerboard pattern - minimal merging possible
        for x in 20..30 {
            for y in 20..30 {
                for z in 20..30 {
                    if (x + y + z) % 2 == 0 {
                        chunk.set(x, y, z, 1);
                    }
                }
            }
        }

        let (_mesh, stats) = mesh_chunk_with_stats(&chunk, 1.0, [0.0, 0.0, 0.0]);

        // Checkerboard has low merge efficiency
        assert!(stats.merge_efficiency < 0.5, "Checkerboard should have low merge efficiency, got {}", stats.merge_efficiency);
    }

    #[test]
    fn mesh_cube_interior_culled() {
        let mut chunk = BinaryChunk::new();
        // Solid 10x10x10 cube
        for x in 20..30 {
            for y in 20..30 {
                for z in 20..30 {
                    chunk.set(x, y, z, 1);
                }
            }
        }

        let (mesh, stats) = mesh_chunk_with_stats(&chunk, 1.0, [0.0, 0.0, 0.0]);

        // Only surface faces should be generated (not interior)
        // 10x10 cube = 6 faces of 100 each = 600 potential faces
        // But with merging, we get 6 quads (one per side)
        assert_eq!(stats.quads_per_face, [1, 1, 1, 1, 1, 1], "Solid cube should have 1 merged quad per face");
        assert_eq!(stats.quad_count, 6);

        // 6 quads = 24 vertices, 12 triangles
        assert_eq!(mesh.vertex_count(), 24);
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn mesh_voxel_size_affects_positions() {
        let mut chunk = BinaryChunk::new();
        chunk.set(1, 1, 1, 1);

        let mesh_small = mesh_chunk(&chunk, 0.1, [0.0, 0.0, 0.0]);
        let mesh_large = mesh_chunk(&chunk, 10.0, [0.0, 0.0, 0.0]);

        // Find max position in each mesh
        let max_small = mesh_small.positions.iter().cloned().fold(0.0f32, f32::max);
        let max_large = mesh_large.positions.iter().cloned().fold(0.0f32, f32::max);

        // Large voxel size should produce larger positions
        assert!(max_large > max_small);
    }
}
