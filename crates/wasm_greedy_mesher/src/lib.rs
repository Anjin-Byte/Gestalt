//! WASM bindings for the greedy mesher.
//!
//! Provides JavaScript-accessible functions for voxel meshing.

use wasm_bindgen::prelude::*;
use greedy_mesher::{
    mesh_chunk, mesh_chunk_with_uvs,
    positions_to_binary_chunk, dense_to_binary_chunk_boxed,
    MaterialId, MeshOutput,
};

/// Mesh result returned to JavaScript.
///
/// Contains vertex data ready for use with Three.js BufferGeometry.
#[wasm_bindgen]
pub struct MeshResult {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    uvs: Vec<f32>,
    material_ids: Vec<u16>,
    has_uvs: bool,
}

#[wasm_bindgen]
impl MeshResult {
    /// Get vertex positions (3 floats per vertex).
    #[wasm_bindgen(getter)]
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }

    /// Get vertex normals (3 floats per vertex).
    #[wasm_bindgen(getter)]
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }

    /// Get triangle indices.
    #[wasm_bindgen(getter)]
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }

    /// Get UV coordinates (2 floats per vertex).
    /// Returns empty array if UVs were not generated.
    #[wasm_bindgen(getter)]
    pub fn uvs(&self) -> Vec<f32> {
        self.uvs.clone()
    }

    /// Get per-vertex material IDs.
    /// Returns empty array if materials were not generated.
    #[wasm_bindgen(getter)]
    pub fn material_ids(&self) -> Vec<u16> {
        self.material_ids.clone()
    }

    /// Number of vertices in the mesh.
    #[wasm_bindgen(getter)]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of triangles in the mesh.
    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether the mesh includes UV coordinates.
    #[wasm_bindgen(getter)]
    pub fn has_uvs(&self) -> bool {
        self.has_uvs
    }

    /// Whether the mesh is empty (no geometry).
    #[wasm_bindgen(getter)]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

impl From<MeshOutput> for MeshResult {
    fn from(output: MeshOutput) -> Self {
        let has_uvs = !output.uvs.is_empty();
        Self {
            positions: output.positions,
            normals: output.normals,
            indices: output.indices,
            uvs: output.uvs,
            material_ids: output.material_ids,
            has_uvs,
        }
    }
}

/// Mesh voxel center positions into optimized geometry.
///
/// Takes an array of voxel center positions (x, y, z triples) and generates
/// an optimized mesh using greedy meshing.
///
/// # Arguments
/// * `positions` - Flat array of voxel positions (x, y, z triples)
/// * `voxel_size` - Size of each voxel in world units
/// * `material_id` - Material ID to assign to all voxels
/// * `origin_x`, `origin_y`, `origin_z` - World position offset
///
/// # Example (JavaScript)
/// ```javascript
/// const positions = new Float32Array([0.5, 0.5, 0.5, 1.5, 0.5, 0.5]);
/// const result = mesh_voxel_positions(positions, 1.0, 1, 0.0, 0.0, 0.0);
/// ```
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
    output.into()
}

/// Mesh voxel positions with UV coordinates and material IDs.
///
/// Same as `mesh_voxel_positions` but includes UV coordinates for texture
/// mapping and per-vertex material IDs for texture atlas lookup.
#[wasm_bindgen]
pub fn mesh_voxel_positions_with_uvs(
    positions: &[f32],
    voxel_size: f32,
    material_id: u16,
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
) -> MeshResult {
    let origin = [origin_x, origin_y, origin_z];
    let chunk = positions_to_binary_chunk(positions, voxel_size, origin, material_id as MaterialId);
    let output = mesh_chunk_with_uvs(&chunk, voxel_size, origin);
    output.into()
}

/// Mesh a dense voxel grid.
///
/// Takes a 3D grid of material IDs (0 = empty) and generates an optimized mesh.
///
/// # Arguments
/// * `voxels` - Flat array of material IDs (0 = empty), X-major order
/// * `width`, `height`, `depth` - Grid dimensions
/// * `voxel_size` - Size of each voxel in world units
/// * `origin_x`, `origin_y`, `origin_z` - World position offset
/// * `generate_uvs` - Whether to generate UV coordinates
///
/// # Example (JavaScript)
/// ```javascript
/// const voxels = new Uint16Array(64 * 64 * 64);
/// voxels.fill(1); // All solid
/// const result = mesh_dense_voxels(voxels, 64, 64, 64, 0.1, 0.0, 0.0, 0.0, true);
/// ```
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
    // Use boxed version to avoid stack overflow in WASM
    let chunk = dense_to_binary_chunk_boxed(voxels, dims);

    let output = if generate_uvs {
        mesh_chunk_with_uvs(&chunk, voxel_size, origin)
    } else {
        mesh_chunk(&chunk, voxel_size, origin)
    };

    output.into()
}

/// Mesh statistics for debugging.
#[wasm_bindgen]
pub struct MeshStats {
    quad_count: usize,
    vertex_count: usize,
    triangle_count: usize,
    merge_efficiency: f32,
}

#[wasm_bindgen]
impl MeshStats {
    #[wasm_bindgen(getter)]
    pub fn quad_count(&self) -> usize {
        self.quad_count
    }

    #[wasm_bindgen(getter)]
    pub fn vertex_count(&self) -> usize {
        self.vertex_count
    }

    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.triangle_count
    }

    #[wasm_bindgen(getter)]
    pub fn merge_efficiency(&self) -> f32 {
        self.merge_efficiency
    }
}

/// Mesh dense voxels and return statistics along with the mesh.
#[wasm_bindgen]
pub fn mesh_dense_voxels_with_stats(
    voxels: &[u16],
    width: u32,
    height: u32,
    depth: u32,
    voxel_size: f32,
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
) -> js_sys::Array {
    let dims = [width as usize, height as usize, depth as usize];
    let origin = [origin_x, origin_y, origin_z];
    // Use boxed version to avoid stack overflow in WASM
    let chunk = dense_to_binary_chunk_boxed(voxels, dims);

    let (output, stats) = greedy_mesher::mesh::mesh_chunk_with_stats(&chunk, voxel_size, origin);

    let mesh_result: MeshResult = output.into();
    let mesh_stats = MeshStats {
        quad_count: stats.quad_count,
        vertex_count: stats.vertex_count,
        triangle_count: stats.triangle_count,
        merge_efficiency: stats.merge_efficiency,
    };

    let result = js_sys::Array::new();
    result.push(&JsValue::from(mesh_result));
    result.push(&JsValue::from(mesh_stats));
    result
}

/// Debug output for greedy mesh visualization.
///
/// Contains the mesh, wireframe lines, per-vertex colors, and statistics.
#[wasm_bindgen]
pub struct MeshDebugResult {
    // Mesh data
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
    // Wireframe line positions (pairs of xyz endpoints)
    wire_positions: Vec<f32>,
    // Per-vertex colors for face direction visualization
    face_colors: Vec<f32>,
    // Per-vertex colors for quad size heatmap
    size_colors: Vec<f32>,
    // Stats
    quad_count: usize,
    vertex_count: usize,
    triangle_count: usize,
    max_possible_quads: usize,
    merge_efficiency: f32,
    triangle_reduction: f32,
    // Per-direction quad counts: [+Y, -Y, +X, -X, +Z, -Z]
    dir_quad_counts: [usize; 6],
    // Per-direction face counts: [+Y, -Y, +X, -X, +Z, -Z]
    dir_face_counts: [usize; 6],
}

#[wasm_bindgen]
impl MeshDebugResult {
    #[wasm_bindgen(getter)]
    pub fn positions(&self) -> Vec<f32> { self.positions.clone() }

    #[wasm_bindgen(getter)]
    pub fn normals(&self) -> Vec<f32> { self.normals.clone() }

    #[wasm_bindgen(getter)]
    pub fn indices(&self) -> Vec<u32> { self.indices.clone() }

    #[wasm_bindgen(getter)]
    pub fn wire_positions(&self) -> Vec<f32> { self.wire_positions.clone() }

    #[wasm_bindgen(getter)]
    pub fn face_colors(&self) -> Vec<f32> { self.face_colors.clone() }

    #[wasm_bindgen(getter)]
    pub fn size_colors(&self) -> Vec<f32> { self.size_colors.clone() }

    #[wasm_bindgen(getter)]
    pub fn quad_count(&self) -> usize { self.quad_count }

    #[wasm_bindgen(getter)]
    pub fn vertex_count(&self) -> usize { self.vertex_count }

    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize { self.triangle_count }

    #[wasm_bindgen(getter)]
    pub fn max_possible_quads(&self) -> usize { self.max_possible_quads }

    #[wasm_bindgen(getter)]
    pub fn merge_efficiency(&self) -> f32 { self.merge_efficiency }

    #[wasm_bindgen(getter)]
    pub fn triangle_reduction(&self) -> f32 { self.triangle_reduction }

    #[wasm_bindgen(getter)]
    pub fn is_empty(&self) -> bool { self.indices.is_empty() }

    /// Get per-direction quad counts as [+Y, -Y, +X, -X, +Z, -Z].
    #[wasm_bindgen(getter)]
    pub fn dir_quad_counts(&self) -> Vec<usize> { self.dir_quad_counts.to_vec() }

    /// Get per-direction face counts as [+Y, -Y, +X, -X, +Z, -Z].
    #[wasm_bindgen(getter)]
    pub fn dir_face_counts(&self) -> Vec<usize> { self.dir_face_counts.to_vec() }
}

/// Mesh dense voxels with full debug output.
///
/// Returns mesh geometry, wireframe lines for quad boundaries,
/// per-vertex colors for visualization modes, and detailed statistics.
#[wasm_bindgen]
pub fn mesh_dense_voxels_debug(
    voxels: &[u16],
    width: u32,
    height: u32,
    depth: u32,
    voxel_size: f32,
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
) -> MeshDebugResult {
    let dims = [width as usize, height as usize, depth as usize];
    let origin = [origin_x, origin_y, origin_z];
    let chunk = dense_to_binary_chunk_boxed(voxels, dims);

    let output = greedy_mesher::mesh::mesh_chunk_debug(&chunk, voxel_size, origin);

    MeshDebugResult {
        positions: output.mesh.positions,
        normals: output.mesh.normals,
        indices: output.mesh.indices,
        wire_positions: output.debug.line_positions,
        face_colors: output.debug.face_colors,
        size_colors: output.debug.size_colors,
        quad_count: output.stats.quad_count,
        vertex_count: output.stats.vertex_count,
        triangle_count: output.stats.triangle_count,
        max_possible_quads: output.stats.max_possible_quads,
        merge_efficiency: output.stats.merge_efficiency,
        triangle_reduction: output.direction_stats.triangle_reduction,
        dir_quad_counts: output.direction_stats.quad_counts,
        dir_face_counts: output.direction_stats.face_counts,
    }
}

// Logging support

thread_local! {
    static LOG_ENABLED: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

/// Enable or disable console logging.
#[wasm_bindgen]
pub fn set_log_enabled(enabled: bool) {
    LOG_ENABLED.with(|flag| flag.set(enabled));
}

#[allow(dead_code)]
fn log(message: &str) {
    if LOG_ENABLED.with(|enabled| enabled.get()) {
        web_sys::console::log_1(&message.into());
    }
}

/// Get the version of the mesher library.
#[wasm_bindgen]
pub fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
