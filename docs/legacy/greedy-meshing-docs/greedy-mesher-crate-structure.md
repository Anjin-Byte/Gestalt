# Greedy Mesher Crate Structure

> **Part of the Voxel Mesh Architecture**
>
> Defines the Rust crate organization for the binary greedy meshing algorithm.
>
> Related documents:
> - [Development Guidelines](development-guidelines.md) - Coding standards
> - [Binary Greedy Mesh Implementation](greedy-mesh-implementation-plan.md) - Algorithm details
> - [ADR-0003](adr/0003-binary-greedy-meshing.md) - Algorithm decision

---

## Overview

The greedy mesher is split into two crates following the existing voxelizer pattern:

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `greedy_mesher` | Core algorithms (pure Rust) | `bytemuck` |
| `wasm_greedy_mesher` | WASM bindings for web | `greedy_mesher`, `wasm-bindgen` |

This separation allows:
- Core algorithms usable in non-WASM contexts (tests, CLI tools)
- Clean WASM boundary with explicit type conversions
- Independent versioning and testing

---

## Directory Structure

```
crates/
├── greedy_mesher/
│   ├── Cargo.toml
│   ├── README.md
│   └── src/
│       ├── lib.rs              # Public exports
│       ├── core.rs             # Type definitions (~100 lines)
│       ├── convert.rs          # Input conversion (~80 lines)
│       ├── cull.rs             # Face culling (~60 lines)
│       ├── merge/
│       │   ├── mod.rs          # Re-exports (~20 lines)
│       │   ├── y_faces.rs      # +Y/-Y merging (~120 lines)
│       │   ├── x_faces.rs      # +X/-X merging (~100 lines)
│       │   └── z_faces.rs      # +Z/-Z merging (~100 lines)
│       ├── expand.rs           # Quad expansion (~150 lines)
│       └── mesh.rs             # Pipeline (~50 lines)
│
└── wasm_greedy_mesher/
    ├── Cargo.toml
    └── src/
        └── lib.rs              # WASM bindings (~200 lines)
```

Total: ~980 lines across 11 files (well under limits)

---

## Core Crate: `greedy_mesher`

### Cargo.toml

```toml
[package]
name = "greedy_mesher"
version = "0.1.0"
edition = "2021"
description = "Binary greedy meshing for 64³ voxel chunks"
license = "MIT"

[dependencies]
bytemuck = { version = "1.16", features = ["derive"] }

[dev-dependencies]
criterion = "0.5"

[[bench]]
name = "meshing"
harness = false
```

### lib.rs

```rust
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
//! chunk.set(32, 32, 32, 1); // Single voxel
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

// Re-export primary types
pub use crate::core::{
    BinaryChunk,
    FaceMasks,
    MeshOutput,
    MaterialId,
    // Constants
    CS_P, CS, CS_P2, CS_P3,
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
    sparse_to_binary_chunks,
};
```

### core.rs

```rust
//! Core type definitions for the greedy mesher.

/// Material identifier (16-bit for texture atlas support)
pub type MaterialId = u16;

/// Chunk size with 1-voxel padding (64)
pub const CS_P: usize = 64;
/// Usable chunk size (62)
pub const CS: usize = 62;
/// Slice size (CS_P × CS_P = 4096)
pub const CS_P2: usize = CS_P * CS_P;
/// Total voxels (CS_P³ = 262144)
pub const CS_P3: usize = CS_P * CS_P * CS_P;

/// Face direction indices
pub const FACE_POS_Y: usize = 0;
pub const FACE_NEG_Y: usize = 1;
pub const FACE_POS_X: usize = 2;
pub const FACE_NEG_X: usize = 3;
pub const FACE_POS_Z: usize = 4;
pub const FACE_NEG_Z: usize = 5;

/// Normal vectors for each face direction
pub const FACE_NORMALS: [[f32; 3]; 6] = [
    [0.0, 1.0, 0.0],   // +Y
    [0.0, -1.0, 0.0],  // -Y
    [1.0, 0.0, 0.0],   // +X
    [-1.0, 0.0, 0.0],  // -X
    [0.0, 0.0, 1.0],   // +Z
    [0.0, 0.0, -1.0],  // -Z
];

/// Binary representation of a voxel chunk.
///
/// Uses column-based bitmasks for efficient face culling.
/// The 1-voxel padding allows neighbor lookups without bounds checks.
#[derive(Clone)]
pub struct BinaryChunk {
    /// Opaque mask: one bit per voxel, organized as Y columns.
    /// `opaque_mask[x * CS_P + z]` contains 64 bits for Y positions.
    pub opaque_mask: [u64; CS_P2],

    /// Material IDs per voxel (16-bit for texture atlas support).
    pub materials: [MaterialId; CS_P3],
}

impl BinaryChunk {
    pub fn new() -> Self {
        Self {
            opaque_mask: [0u64; CS_P2],
            materials: [0u16; CS_P3],
        }
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, material: MaterialId) {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P);
        let column_idx = x * CS_P + z;
        self.opaque_mask[column_idx] |= 1u64 << y;
        self.materials[x * CS_P2 + y * CS_P + z] = material;
    }

    #[inline]
    pub fn clear(&mut self, x: usize, y: usize, z: usize) {
        debug_assert!(x < CS_P && y < CS_P && z < CS_P);
        let column_idx = x * CS_P + z;
        self.opaque_mask[column_idx] &= !(1u64 << y);
        self.materials[x * CS_P2 + y * CS_P + z] = 0;
    }

    #[inline]
    pub fn is_solid(&self, x: usize, y: usize, z: usize) -> bool {
        let column_idx = x * CS_P + z;
        (self.opaque_mask[column_idx] >> y) & 1 != 0
    }

    #[inline]
    pub fn get_material(&self, x: usize, y: usize, z: usize) -> MaterialId {
        self.materials[x * CS_P2 + y * CS_P + z]
    }
}

impl Default for BinaryChunk {
    fn default() -> Self {
        Self::new()
    }
}

/// Face masks for all 6 directions after culling.
#[derive(Clone)]
pub struct FaceMasks {
    /// `masks[face * CS_P2 + x * CS_P + z]` = visible faces in Y column
    pub masks: [u64; 6 * CS_P2],
}

impl FaceMasks {
    pub fn new() -> Self {
        Self { masks: [0u64; 6 * CS_P2] }
    }

    #[inline]
    pub fn get(&self, face: usize, x: usize, z: usize) -> u64 {
        self.masks[face * CS_P2 + x * CS_P + z]
    }

    #[inline]
    pub fn set(&mut self, face: usize, x: usize, z: usize, mask: u64) {
        self.masks[face * CS_P2 + x * CS_P + z] = mask;
    }
}

impl Default for FaceMasks {
    fn default() -> Self {
        Self::new()
    }
}

/// Output mesh data ready for GPU buffers.
#[derive(Default, Clone)]
pub struct MeshOutput {
    /// Vertex positions (3 floats per vertex)
    pub positions: Vec<f32>,
    /// Vertex normals (3 floats per vertex)
    pub normals: Vec<f32>,
    /// Triangle indices
    pub indices: Vec<u32>,
    /// UV coordinates (2 floats per vertex, optional)
    pub uvs: Vec<f32>,
    /// Per-vertex material IDs (optional, for shader lookup)
    pub material_ids: Vec<MaterialId>,
}

impl MeshOutput {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn has_uvs(&self) -> bool {
        !self.uvs.is_empty()
    }

    pub fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.indices.clear();
        self.uvs.clear();
        self.material_ids.clear();
    }
}

/// Pack quad data into 64 bits.
///
/// Layout:
/// - Bits 0-5: X (0-63)
/// - Bits 6-11: Y (0-63)
/// - Bits 12-17: Z (0-63)
/// - Bits 18-23: Width (1-64)
/// - Bits 24-29: Height (1-64)
/// - Bits 30-31: Reserved
/// - Bits 32-47: Material ID (0-65535)
/// - Bits 48-63: Reserved
#[inline]
pub fn pack_quad(x: u32, y: u32, z: u32, w: u32, h: u32, material: MaterialId) -> u64 {
    ((material as u64) << 32)
        | ((h as u64) << 24)
        | ((w as u64) << 18)
        | ((z as u64) << 12)
        | ((y as u64) << 6)
        | (x as u64)
}

/// Unpack quad data from 64 bits.
#[inline]
pub fn unpack_quad(quad: u64) -> (u32, u32, u32, u32, u32, MaterialId) {
    let x = (quad & 0x3F) as u32;
    let y = ((quad >> 6) & 0x3F) as u32;
    let z = ((quad >> 12) & 0x3F) as u32;
    let w = ((quad >> 18) & 0x3F) as u32;
    let h = ((quad >> 24) & 0x3F) as u32;
    let material = ((quad >> 32) & 0xFFFF) as MaterialId;
    (x, y, z, w, h, material)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let quad = pack_quad(10, 20, 30, 5, 8, 1234);
        let (x, y, z, w, h, mat) = unpack_quad(quad);
        assert_eq!((x, y, z, w, h, mat), (10, 20, 30, 5, 8, 1234));
    }

    #[test]
    fn chunk_set_get() {
        let mut chunk = BinaryChunk::new();
        chunk.set(10, 20, 30, 42);

        assert!(chunk.is_solid(10, 20, 30));
        assert_eq!(chunk.get_material(10, 20, 30), 42);
        assert!(!chunk.is_solid(10, 20, 31));
    }
}
```

### convert.rs

```rust
//! Input conversion functions to BinaryChunk format.

use crate::core::{BinaryChunk, MaterialId, CS_P, CS};

/// Convert voxel center positions to binary chunk.
///
/// Positions are world-space (x, y, z) tuples.
/// Voxels outside chunk bounds are ignored.
pub fn positions_to_binary_chunk(
    positions: &[f32],
    voxel_size: f32,
    chunk_origin: [f32; 3],
    material: MaterialId,
) -> BinaryChunk {
    let mut chunk = BinaryChunk::new();
    let inv_size = 1.0 / voxel_size;

    for pos in positions.chunks_exact(3) {
        let lx = ((pos[0] - chunk_origin[0]) * inv_size) as i32 + 1;
        let ly = ((pos[1] - chunk_origin[1]) * inv_size) as i32 + 1;
        let lz = ((pos[2] - chunk_origin[2]) * inv_size) as i32 + 1;

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
/// Input is material ID per voxel (0 = empty), stored in X-Y-Z order.
pub fn dense_to_binary_chunk(
    voxels: &[MaterialId],
    dims: [usize; 3],
) -> BinaryChunk {
    let mut chunk = BinaryChunk::new();
    let [dx, dy, dz] = dims;

    for z in 0..dz.min(CS) {
        for y in 0..dy.min(CS) {
            for x in 0..dx.min(CS) {
                let src_idx = z * dy * dx + y * dx + x;
                if src_idx < voxels.len() {
                    let material = voxels[src_idx];
                    if material != 0 {
                        chunk.set(x + 1, y + 1, z + 1, material);
                    }
                }
            }
        }
    }

    chunk
}

/// Convert sparse brick output to binary chunks.
///
/// This bridges the GPU voxelizer's sparse output format
/// to the mesher's dense chunk format.
///
/// Returns chunks with their world-space origins.
pub fn sparse_to_binary_chunks(
    brick_dim: u32,
    brick_origins: &[[u32; 3]],
    occupancy: &[u32],
    owner_ids: Option<&[u32]>,
    grid_dims: [u32; 3],
    material_map: impl Fn(u32) -> MaterialId,
) -> Vec<(BinaryChunk, [i32; 3])> {
    let chunk_size = CS as u32;
    let mut chunks: std::collections::HashMap<[i32; 3], BinaryChunk> = Default::default();

    let brick_voxels = brick_dim * brick_dim * brick_dim;
    let words_per_brick = ((brick_voxels + 31) / 32) as usize;

    for (brick_idx, origin) in brick_origins.iter().enumerate() {
        let base_offset = brick_idx * words_per_brick;

        for local_z in 0..brick_dim {
            for local_y in 0..brick_dim {
                for local_x in 0..brick_dim {
                    let bit_idx = local_z * brick_dim * brick_dim + local_y * brick_dim + local_x;
                    let word_idx = (bit_idx / 32) as usize;
                    let bit_pos = bit_idx % 32;

                    if base_offset + word_idx >= occupancy.len() {
                        continue;
                    }

                    let is_set = (occupancy[base_offset + word_idx] >> bit_pos) & 1 != 0;
                    if !is_set {
                        continue;
                    }

                    // Global voxel position
                    let gx = origin[0] + local_x;
                    let gy = origin[1] + local_y;
                    let gz = origin[2] + local_z;

                    // Chunk coordinate
                    let cx = (gx / chunk_size) as i32;
                    let cy = (gy / chunk_size) as i32;
                    let cz = (gz / chunk_size) as i32;

                    // Local position within chunk (+1 for padding)
                    let lx = ((gx % chunk_size) + 1) as usize;
                    let ly = ((gy % chunk_size) + 1) as usize;
                    let lz = ((gz % chunk_size) + 1) as usize;

                    // Get or create chunk
                    let chunk = chunks.entry([cx, cy, cz]).or_insert_with(BinaryChunk::new);

                    // Determine material
                    let material = if let Some(owners) = owner_ids {
                        let owner_idx = base_offset + word_idx;
                        if owner_idx < owners.len() {
                            material_map(owners[owner_idx])
                        } else {
                            1
                        }
                    } else {
                        1
                    };

                    chunk.set(lx, ly, lz, material);
                }
            }
        }
    }

    chunks.into_iter().collect()
}
```

### cull.rs

```rust
//! Bitwise face culling.

use crate::core::{BinaryChunk, FaceMasks, CS_P, CS, FACE_POS_Y, FACE_NEG_Y, FACE_POS_X, FACE_NEG_X, FACE_POS_Z, FACE_NEG_Z};

/// Generate face masks using bitwise neighbor culling.
///
/// A face is visible if the voxel is solid AND the neighbor is empty.
pub fn cull_faces(chunk: &BinaryChunk, masks: &mut FaceMasks) {
    let usable_mask = (1u64 << CS) - 1;

    for x in 1..CS_P - 1 {
        let x_cs_p = x * CS_P;

        for z in 1..CS_P - 1 {
            let column_idx = x_cs_p + z;
            let column = chunk.opaque_mask[column_idx];

            if column == 0 {
                continue;
            }

            // +Y: visible where solid AND y+1 is empty
            let pos_y = column & !(column >> 1);
            masks.set(FACE_POS_Y, x, z, (pos_y >> 1) & usable_mask);

            // -Y: visible where solid AND y-1 is empty
            let neg_y = column & !(column << 1);
            masks.set(FACE_NEG_Y, x, z, (neg_y >> 1) & usable_mask);

            // +X: compare with x+1 column
            let neighbor_pos_x = chunk.opaque_mask[(x + 1) * CS_P + z];
            let pos_x = column & !neighbor_pos_x;
            masks.set(FACE_POS_X, x, z, (pos_x >> 1) & usable_mask);

            // -X: compare with x-1 column
            let neighbor_neg_x = chunk.opaque_mask[(x - 1) * CS_P + z];
            let neg_x = column & !neighbor_neg_x;
            masks.set(FACE_NEG_X, x, z, (neg_x >> 1) & usable_mask);

            // +Z: compare with z+1 column
            let neighbor_pos_z = chunk.opaque_mask[x_cs_p + z + 1];
            let pos_z = column & !neighbor_pos_z;
            masks.set(FACE_POS_Z, x, z, (pos_z >> 1) & usable_mask);

            // -Z: compare with z-1 column
            let neighbor_neg_z = chunk.opaque_mask[x_cs_p + z - 1];
            let neg_z = column & !neighbor_neg_z;
            masks.set(FACE_NEG_Z, x, z, (neg_z >> 1) & usable_mask);
        }
    }
}
```

### merge/mod.rs

```rust
//! Greedy merge algorithms for each face direction.

mod y_faces;
mod x_faces;
mod z_faces;

pub use y_faces::greedy_merge_y_faces;
pub use x_faces::greedy_merge_x_faces;
pub use z_faces::greedy_merge_z_faces;
```

### mesh.rs

```rust
//! Main meshing pipeline.

use crate::core::{BinaryChunk, FaceMasks, MeshOutput, FACE_POS_Y, FACE_NEG_Y, FACE_POS_X, FACE_NEG_X, FACE_POS_Z, FACE_NEG_Z};
use crate::cull::cull_faces;
use crate::merge::{greedy_merge_y_faces, greedy_merge_x_faces, greedy_merge_z_faces};
use crate::expand::{expand_quads, expand_quads_with_uvs};

/// Mesh a binary chunk into geometry (positions, normals, indices).
pub fn mesh_chunk(chunk: &BinaryChunk, voxel_size: f32, origin: [f32; 3]) -> MeshOutput {
    let mut masks = FaceMasks::new();
    cull_faces(chunk, &mut masks);

    let mut packed_quads: [Vec<u64>; 6] = Default::default();

    greedy_merge_y_faces(FACE_POS_Y, chunk, &masks, &mut packed_quads[FACE_POS_Y]);
    greedy_merge_y_faces(FACE_NEG_Y, chunk, &masks, &mut packed_quads[FACE_NEG_Y]);
    greedy_merge_x_faces(FACE_POS_X, chunk, &masks, &mut packed_quads[FACE_POS_X]);
    greedy_merge_x_faces(FACE_NEG_X, chunk, &masks, &mut packed_quads[FACE_NEG_X]);
    greedy_merge_z_faces(FACE_POS_Z, chunk, &masks, &mut packed_quads[FACE_POS_Z]);
    greedy_merge_z_faces(FACE_NEG_Z, chunk, &masks, &mut packed_quads[FACE_NEG_Z]);

    expand_quads(&packed_quads, voxel_size, origin)
}

/// Mesh with UV coordinates and per-vertex material IDs.
pub fn mesh_chunk_with_uvs(chunk: &BinaryChunk, voxel_size: f32, origin: [f32; 3]) -> MeshOutput {
    let mut masks = FaceMasks::new();
    cull_faces(chunk, &mut masks);

    let mut packed_quads: [Vec<u64>; 6] = Default::default();

    greedy_merge_y_faces(FACE_POS_Y, chunk, &masks, &mut packed_quads[FACE_POS_Y]);
    greedy_merge_y_faces(FACE_NEG_Y, chunk, &masks, &mut packed_quads[FACE_NEG_Y]);
    greedy_merge_x_faces(FACE_POS_X, chunk, &masks, &mut packed_quads[FACE_POS_X]);
    greedy_merge_x_faces(FACE_NEG_X, chunk, &masks, &mut packed_quads[FACE_NEG_X]);
    greedy_merge_z_faces(FACE_POS_Z, chunk, &masks, &mut packed_quads[FACE_POS_Z]);
    greedy_merge_z_faces(FACE_NEG_Z, chunk, &masks, &mut packed_quads[FACE_NEG_Z]);

    expand_quads_with_uvs(&packed_quads, voxel_size, origin)
}
```

---

## WASM Crate: `wasm_greedy_mesher`

### Cargo.toml

```toml
[package]
name = "wasm_greedy_mesher"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "0.2"
js-sys = "0.3"
greedy_mesher = { path = "../greedy_mesher" }
bytemuck = { version = "1.16", features = ["derive"] }

[dependencies.web-sys]
version = "0.3"
features = ["console"]

[profile.release]
opt-level = "z"
lto = true
```

### lib.rs

```rust
use wasm_bindgen::prelude::*;
use greedy_mesher::{
    mesh_chunk, mesh_chunk_with_uvs,
    positions_to_binary_chunk, dense_to_binary_chunk,
    MaterialId,
};

/// Mesh result returned to JavaScript.
#[wasm_bindgen]
pub struct MeshResult {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    uvs: Option<Vec<f32>>,
    material_ids: Option<Vec<u16>>,
}

#[wasm_bindgen]
impl MeshResult {
    #[wasm_bindgen(getter)]
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn uvs(&self) -> Option<Vec<f32>> {
        self.uvs.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn material_ids(&self) -> Option<Vec<u16>> {
        self.material_ids.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    #[wasm_bindgen(getter)]
    pub fn has_uvs(&self) -> bool {
        self.uvs.is_some()
    }
}

/// Mesh voxel center positions.
#[wasm_bindgen]
pub fn mesh_voxel_positions(
    positions: &[f32],
    voxel_size: f32,
    material_id: u16,
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
) -> MeshResult {
    let origin = [origin_x, origin_y, origin_z];
    let chunk = positions_to_binary_chunk(positions, voxel_size, origin, material_id as MaterialId);
    let output = mesh_chunk(&chunk, voxel_size, origin);

    MeshResult {
        positions: output.positions,
        normals: output.normals,
        indices: output.indices,
        uvs: None,
        material_ids: None,
    }
}

/// Mesh dense voxel grid.
#[wasm_bindgen]
pub fn mesh_dense_voxels(
    voxels: &[u16],
    width: u32,
    height: u32,
    depth: u32,
    voxel_size: f32,
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
    generate_uvs: bool,
) -> MeshResult {
    let dims = [width as usize, height as usize, depth as usize];
    let origin = [origin_x, origin_y, origin_z];
    let chunk = dense_to_binary_chunk(voxels, dims);

    if generate_uvs {
        let output = mesh_chunk_with_uvs(&chunk, voxel_size, origin);
        MeshResult {
            positions: output.positions,
            normals: output.normals,
            indices: output.indices,
            uvs: Some(output.uvs),
            material_ids: Some(output.material_ids),
        }
    } else {
        let output = mesh_chunk(&chunk, voxel_size, origin);
        MeshResult {
            positions: output.positions,
            normals: output.normals,
            indices: output.indices,
            uvs: None,
            material_ids: None,
        }
    }
}

/// Enable/disable console logging.
#[wasm_bindgen]
pub fn set_log_enabled(enabled: bool) {
    LOG_ENABLED.with(|flag| flag.set(enabled));
}

thread_local! {
    static LOG_ENABLED: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

fn log(message: &str) {
    if LOG_ENABLED.with(|enabled| enabled.get()) {
        web_sys::console::log_1(&message.into());
    }
}
```

---

## Build Integration

### package.json scripts

```json
{
  "scripts": {
    "build:wasm:mesher": "cd crates/wasm_greedy_mesher && wasm-pack build --target web --out-dir ../../apps/web/src/wasm/wasm_greedy_mesher",
    "build:wasm": "npm run build:wasm:voxelizer && npm run build:wasm:mesher"
  }
}
```

### TypeScript Types

After `wasm-pack build`, types are generated at:
```
apps/web/src/wasm/wasm_greedy_mesher/
├── wasm_greedy_mesher.d.ts
├── wasm_greedy_mesher.js
├── wasm_greedy_mesher_bg.wasm
└── wasm_greedy_mesher_bg.wasm.d.ts
```

Usage:
```typescript
import init, { mesh_voxel_positions, mesh_dense_voxels, MeshResult } from '@/wasm/wasm_greedy_mesher';

await init();

const result: MeshResult = mesh_dense_voxels(
    voxelData,
    64, 64, 64,
    1.0,
    0.0, 0.0, 0.0,
    true // generate UVs
);

const geometry = new THREE.BufferGeometry();
geometry.setAttribute('position', new THREE.Float32BufferAttribute(result.positions, 3));
geometry.setAttribute('normal', new THREE.Float32BufferAttribute(result.normals, 3));
geometry.setIndex(new THREE.Uint32BufferAttribute(result.indices, 1));

if (result.has_uvs) {
    geometry.setAttribute('uv', new THREE.Float32BufferAttribute(result.uvs!, 2));
}
```

---

## Testing

### Unit Tests (in Rust)

```bash
cd crates/greedy_mesher
cargo test
```

### Benchmarks

```bash
cd crates/greedy_mesher
cargo bench
```

### WASM Tests

```bash
cd crates/wasm_greedy_mesher
wasm-pack test --headless --chrome
```

---

## Summary

| File | Purpose | Est. Lines |
|------|---------|------------|
| `greedy_mesher/src/lib.rs` | Exports | 40 |
| `greedy_mesher/src/core.rs` | Types | 150 |
| `greedy_mesher/src/convert.rs` | Input conversion | 100 |
| `greedy_mesher/src/cull.rs` | Face culling | 60 |
| `greedy_mesher/src/merge/*.rs` | Greedy merge | 350 |
| `greedy_mesher/src/expand.rs` | Quad expansion | 180 |
| `greedy_mesher/src/mesh.rs` | Pipeline | 50 |
| `wasm_greedy_mesher/src/lib.rs` | WASM bindings | 150 |
| **Total** | | ~1080 |

All files stay well under the 300-500 line limits from development guidelines.
