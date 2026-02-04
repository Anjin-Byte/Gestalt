//! Binary greedy meshing for 64³ voxel chunks.
//!
//! This crate provides high-performance voxel meshing using bitwise
//! operations to process 64 voxels per instruction.
//!
//! # Example
//!
//! ```
//! use greedy_mesher::{BinaryChunk, mesh_chunk};
//!
//! let mut chunk = BinaryChunk::new();
//! chunk.set(32, 32, 32, 1); // Single voxel at center
//!
//! let mesh = mesh_chunk(&chunk, 1.0, [0.0, 0.0, 0.0]);
//! assert_eq!(mesh.triangle_count(), 12); // Cube = 6 faces × 2 triangles
//! ```

pub mod core;
pub mod convert;
pub mod cull;
pub mod merge;
pub mod expand;
pub mod mesh;
pub mod chunk;

// Re-export primary types
pub use crate::core::{
    BinaryChunk,
    FaceMasks,
    MeshOutput,
    MaterialId,
    // Constants
    CS_P, CS, CS_P2, CS_P3,
    MATERIAL_EMPTY, MATERIAL_DEFAULT,
    FACE_POS_Y, FACE_NEG_Y, FACE_POS_X, FACE_NEG_X, FACE_POS_Z, FACE_NEG_Z,
    FACE_NORMALS,
    // Quad packing
    pack_quad, unpack_quad,
};

// Re-export main entry points
pub use crate::mesh::{mesh_chunk, mesh_chunk_with_uvs};
pub use crate::convert::{
    positions_to_binary_chunk,
    dense_to_binary_chunk,
    dense_to_binary_chunk_boxed,
};
