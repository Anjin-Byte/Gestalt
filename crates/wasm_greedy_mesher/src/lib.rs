//! WASM bindings for the greedy mesher.
//!
//! Provides JavaScript-accessible functions for voxel meshing.

use wasm_bindgen::prelude::*;
use greedy_mesher::{
    mesh_chunk, mesh_chunk_with_uvs,
    positions_to_binary_chunk, dense_to_binary_chunk_boxed,
    MaterialId, MeshOutput,
};
use greedy_mesher::chunk::{
    ChunkManager, ChunkCoord,
    FrameStats, ChunkDebugInfo,
    RebuildConfig, MemoryBudget,
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

// =========================================================================
// WasmChunkManager - Multi-chunk voxel world manager
// =========================================================================

/// WASM-accessible chunk manager for multi-chunk voxel worlds.
///
/// Wraps the Rust ChunkManager, exposing chunk creation, voxel editing,
/// frame-budgeted rebuild, mesh swap, and LRU eviction to JavaScript.
#[wasm_bindgen]
pub struct WasmChunkManager {
    inner: ChunkManager,
}

#[wasm_bindgen]
impl WasmChunkManager {
    // =====================================================================
    // Construction
    // =====================================================================

    /// Create a new chunk manager with default configuration.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { inner: ChunkManager::new() }
    }

    /// Create with custom rebuild configuration.
    pub fn with_config(
        max_chunks_per_frame: usize,
        max_time_ms: f64,
        voxel_size: f32,
    ) -> Self {
        Self {
            inner: ChunkManager::with_config(RebuildConfig {
                max_chunks_per_frame,
                max_time_per_frame_ms: max_time_ms,
                voxel_size,
            }),
        }
    }

    /// Create with custom rebuild configuration and memory budget.
    pub fn with_budget(
        max_chunks_per_frame: usize,
        max_time_ms: f64,
        voxel_size: f32,
        budget_max_bytes: usize,
        high_watermark: f32,
        low_watermark: f32,
        min_chunks: usize,
    ) -> Self {
        Self {
            inner: ChunkManager::with_budget(
                RebuildConfig {
                    max_chunks_per_frame,
                    max_time_per_frame_ms: max_time_ms,
                    voxel_size,
                },
                MemoryBudget {
                    max_bytes: budget_max_bytes,
                    high_watermark,
                    low_watermark,
                    min_chunks,
                },
            ),
        }
    }

    // =====================================================================
    // Voxel Editing
    // =====================================================================

    /// Set a voxel at a world position.
    pub fn set_voxel(&mut self, wx: f32, wy: f32, wz: f32, material: u16) {
        self.inner.set_voxel([wx, wy, wz], material);
    }

    /// Set a voxel at integer voxel coordinates.
    pub fn set_voxel_at(&mut self, vx: i32, vy: i32, vz: i32, material: u16) {
        self.inner.set_voxel_at([vx, vy, vz], material);
    }

    /// Batch edit voxels. `edits` is a flat array: [wx, wy, wz, material, ...].
    /// Each edit is 4 floats: 3 for world position + 1 cast to u16 material ID.
    pub fn set_voxels_batch(&mut self, edits: &[f32]) {
        let batch: Vec<([f32; 3], MaterialId)> = edits
            .chunks_exact(4)
            .map(|c| ([c[0], c[1], c[2]], c[3] as MaterialId))
            .collect();
        self.inner.set_voxels_batch(&batch);
    }

    /// Get material at world position.
    pub fn get_voxel(&self, wx: f32, wy: f32, wz: f32) -> u16 {
        self.inner.get_voxel([wx, wy, wz])
    }

    // =====================================================================
    // Frame Update
    // =====================================================================

    /// Run one full frame update: rebuild + swap + evict.
    pub fn update(&mut self, cam_x: f32, cam_y: f32, cam_z: f32) -> WasmFrameStats {
        let stats = self.inner.update([cam_x, cam_y, cam_z]);
        WasmFrameStats::from(stats)
    }

    // =====================================================================
    // Swapped/Evicted Coord Retrieval
    // =====================================================================

    /// Get coords that received new meshes in the last update.
    /// Returns flat i32 array: [x0, y0, z0, x1, y1, z1, ...].
    pub fn last_swapped_coords(&self) -> Vec<i32> {
        self.inner.last_swapped_coords()
            .iter()
            .flat_map(|c| [c.x, c.y, c.z])
            .collect()
    }

    /// Get coords that were evicted in the last update.
    /// Returns flat i32 array: [x0, y0, z0, x1, y1, z1, ...].
    pub fn last_evicted_coords(&self) -> Vec<i32> {
        self.inner.last_evicted_coords()
            .iter()
            .flat_map(|c| [c.x, c.y, c.z])
            .collect()
    }

    // =====================================================================
    // Mesh Data Extraction
    // =====================================================================

    /// Get mesh data for a specific chunk.
    /// Returns MeshResult or JsValue::NULL if the chunk has no mesh.
    pub fn get_chunk_mesh(&self, cx: i32, cy: i32, cz: i32) -> JsValue {
        let coord = ChunkCoord::new(cx, cy, cz);
        let Some(chunk) = self.inner.get_chunk(coord) else {
            return JsValue::NULL;
        };
        let Some(mesh) = chunk.mesh.as_ref() else {
            return JsValue::NULL;
        };

        JsValue::from(MeshResult {
            positions: mesh.positions.clone(),
            normals: mesh.normals.clone(),
            indices: mesh.indices.clone(),
            uvs: mesh.uvs.clone(),
            material_ids: mesh.material_ids.clone(),
            has_uvs: !mesh.uvs.is_empty(),
        })
    }

    /// Get the data version for a chunk (for staleness detection).
    pub fn get_chunk_version(&self, cx: i32, cy: i32, cz: i32) -> u64 {
        let coord = ChunkCoord::new(cx, cy, cz);
        self.inner.get_chunk(coord)
            .map(|c| c.data_version)
            .unwrap_or(0)
    }

    // =====================================================================
    // Memory Budget
    // =====================================================================

    /// Set memory budget configuration.
    pub fn set_budget(
        &mut self,
        max_bytes: usize,
        high_watermark: f32,
        low_watermark: f32,
        min_chunks: usize,
    ) {
        self.inner.set_budget(MemoryBudget {
            max_bytes,
            high_watermark,
            low_watermark,
            min_chunks,
        });
    }

    /// Get current memory usage in bytes.
    pub fn memory_usage_bytes(&self) -> usize {
        self.inner.memory_usage_bytes()
    }

    /// Check if memory budget is exceeded.
    pub fn is_over_budget(&self) -> bool {
        self.inner.is_over_budget()
    }

    /// Get total chunk count.
    pub fn chunk_count(&self) -> usize {
        self.inner.chunk_count()
    }

    // =====================================================================
    // Chunk Management
    // =====================================================================

    /// Check if a chunk exists.
    pub fn has_chunk(&self, cx: i32, cy: i32, cz: i32) -> bool {
        self.inner.has_chunk(ChunkCoord::new(cx, cy, cz))
    }

    /// Remove a specific chunk. Returns true if it existed.
    pub fn remove_chunk(&mut self, cx: i32, cy: i32, cz: i32) -> bool {
        self.inner.remove_chunk(ChunkCoord::new(cx, cy, cz)).is_some()
    }

    /// Record a chunk access for LRU tracking.
    pub fn touch_chunk(&mut self, cx: i32, cy: i32, cz: i32) {
        self.inner.touch_chunk(ChunkCoord::new(cx, cy, cz));
    }

    /// Clear all chunks and reset state.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    // =====================================================================
    // Batch Operations
    // =====================================================================

    /// Populate chunks from a dense voxel grid of arbitrary dimensions.
    ///
    /// Clears all existing chunks, then iterates the grid in native Rust
    /// distributing voxels into correct chunks. One WASM call replaces
    /// width×height×depth individual set_voxel_at calls.
    ///
    /// Grid is X-major order: `voxels[x + y * width + z * width * height]`
    pub fn populate_dense(
        &mut self,
        voxels: &[u16],
        width: u32,
        height: u32,
        depth: u32,
    ) {
        self.inner.populate_dense(
            voxels,
            width as usize,
            height as usize,
            depth as usize,
        );
    }

    /// Force-rebuild all dirty chunks (ignores frame budget).
    ///
    /// Returns the number of chunks rebuilt. Calls swap_pending_meshes
    /// internally, so last_swapped_coords() is populated afterward.
    pub fn rebuild_all_dirty(&mut self) -> usize {
        self.inner.rebuild_all_dirty([0.0, 0.0, 0.0])
    }

    // =====================================================================
    // Debug
    // =====================================================================

    /// Get debug info about the chunk system.
    pub fn debug_info(&self) -> WasmChunkDebugInfo {
        WasmChunkDebugInfo::from(self.inner.debug_info())
    }

    /// Get the voxel size from configuration.
    pub fn voxel_size(&self) -> f32 {
        self.inner.voxel_size()
    }
}

// =========================================================================
// WasmFrameStats - Frame statistics returned from update()
// =========================================================================

/// Frame statistics returned from WasmChunkManager::update().
#[wasm_bindgen]
pub struct WasmFrameStats {
    // Rebuild
    chunks_rebuilt: usize,
    triangles_generated: usize,
    vertices_generated: usize,
    rebuild_elapsed_ms: f64,
    queue_remaining: usize,
    time_budget_exceeded: bool,
    chunk_limit_reached: bool,
    // Swap
    meshes_swapped: usize,
    meshes_disposed: usize,
    version_conflicts: usize,
    // Eviction
    chunks_evicted: usize,
    bytes_freed: usize,
    // Summary
    total_chunks: usize,
    chunks_with_mesh: usize,
    dirty_chunks: usize,
}

#[wasm_bindgen]
impl WasmFrameStats {
    #[wasm_bindgen(getter)] pub fn chunks_rebuilt(&self) -> usize { self.chunks_rebuilt }
    #[wasm_bindgen(getter)] pub fn triangles_generated(&self) -> usize { self.triangles_generated }
    #[wasm_bindgen(getter)] pub fn vertices_generated(&self) -> usize { self.vertices_generated }
    #[wasm_bindgen(getter)] pub fn rebuild_elapsed_ms(&self) -> f64 { self.rebuild_elapsed_ms }
    #[wasm_bindgen(getter)] pub fn queue_remaining(&self) -> usize { self.queue_remaining }
    #[wasm_bindgen(getter)] pub fn time_budget_exceeded(&self) -> bool { self.time_budget_exceeded }
    #[wasm_bindgen(getter)] pub fn chunk_limit_reached(&self) -> bool { self.chunk_limit_reached }
    #[wasm_bindgen(getter)] pub fn meshes_swapped(&self) -> usize { self.meshes_swapped }
    #[wasm_bindgen(getter)] pub fn meshes_disposed(&self) -> usize { self.meshes_disposed }
    #[wasm_bindgen(getter)] pub fn version_conflicts(&self) -> usize { self.version_conflicts }
    #[wasm_bindgen(getter)] pub fn chunks_evicted(&self) -> usize { self.chunks_evicted }
    #[wasm_bindgen(getter)] pub fn bytes_freed(&self) -> usize { self.bytes_freed }
    #[wasm_bindgen(getter)] pub fn total_chunks(&self) -> usize { self.total_chunks }
    #[wasm_bindgen(getter)] pub fn chunks_with_mesh(&self) -> usize { self.chunks_with_mesh }
    #[wasm_bindgen(getter)] pub fn dirty_chunks(&self) -> usize { self.dirty_chunks }
}

impl From<FrameStats> for WasmFrameStats {
    fn from(s: FrameStats) -> Self {
        Self {
            chunks_rebuilt: s.rebuild.chunks_rebuilt,
            triangles_generated: s.rebuild.triangles_generated,
            vertices_generated: s.rebuild.vertices_generated,
            rebuild_elapsed_ms: s.rebuild.elapsed_ms,
            queue_remaining: s.rebuild.queue_remaining,
            time_budget_exceeded: s.rebuild.time_budget_exceeded,
            chunk_limit_reached: s.rebuild.chunk_limit_reached,
            meshes_swapped: s.swap.meshes_swapped,
            meshes_disposed: s.swap.meshes_disposed,
            version_conflicts: s.swap.version_conflicts,
            chunks_evicted: s.eviction.chunks_evicted,
            bytes_freed: s.eviction.bytes_freed,
            total_chunks: s.total_chunks,
            chunks_with_mesh: s.chunks_with_mesh,
            dirty_chunks: s.dirty_chunks,
        }
    }
}

// =========================================================================
// WasmChunkDebugInfo - Debug information wrapper
// =========================================================================

/// Debug information about the chunk system state.
#[wasm_bindgen]
pub struct WasmChunkDebugInfo {
    total_chunks: usize,
    clean_chunks: usize,
    dirty_chunks: usize,
    meshing_chunks: usize,
    ready_to_swap_chunks: usize,
    queue_size: usize,
    total_triangles: usize,
    total_vertices: usize,
    voxel_memory_bytes: usize,
    mesh_memory_bytes: usize,
    budget_max_bytes: usize,
    budget_usage_percent: f32,
    budget_exceeded: bool,
}

#[wasm_bindgen]
impl WasmChunkDebugInfo {
    #[wasm_bindgen(getter)] pub fn total_chunks(&self) -> usize { self.total_chunks }
    #[wasm_bindgen(getter)] pub fn clean_chunks(&self) -> usize { self.clean_chunks }
    #[wasm_bindgen(getter)] pub fn dirty_chunks(&self) -> usize { self.dirty_chunks }
    #[wasm_bindgen(getter)] pub fn meshing_chunks(&self) -> usize { self.meshing_chunks }
    #[wasm_bindgen(getter)] pub fn ready_to_swap_chunks(&self) -> usize { self.ready_to_swap_chunks }
    #[wasm_bindgen(getter)] pub fn queue_size(&self) -> usize { self.queue_size }
    #[wasm_bindgen(getter)] pub fn total_triangles(&self) -> usize { self.total_triangles }
    #[wasm_bindgen(getter)] pub fn total_vertices(&self) -> usize { self.total_vertices }
    #[wasm_bindgen(getter)] pub fn voxel_memory_bytes(&self) -> usize { self.voxel_memory_bytes }
    #[wasm_bindgen(getter)] pub fn mesh_memory_bytes(&self) -> usize { self.mesh_memory_bytes }
    #[wasm_bindgen(getter)] pub fn budget_max_bytes(&self) -> usize { self.budget_max_bytes }
    #[wasm_bindgen(getter)] pub fn budget_usage_percent(&self) -> f32 { self.budget_usage_percent }
    #[wasm_bindgen(getter)] pub fn budget_exceeded(&self) -> bool { self.budget_exceeded }
}

impl From<ChunkDebugInfo> for WasmChunkDebugInfo {
    fn from(info: ChunkDebugInfo) -> Self {
        Self {
            total_chunks: info.total_chunks,
            clean_chunks: info.clean_chunks,
            dirty_chunks: info.dirty_chunks,
            meshing_chunks: info.meshing_chunks,
            ready_to_swap_chunks: info.ready_to_swap_chunks,
            queue_size: info.queue_size,
            total_triangles: info.total_triangles,
            total_vertices: info.total_vertices,
            voxel_memory_bytes: info.voxel_memory_bytes,
            mesh_memory_bytes: info.mesh_memory_bytes,
            budget_max_bytes: info.budget_max_bytes,
            budget_usage_percent: info.budget_usage_percent,
            budget_exceeded: info.budget_exceeded,
        }
    }
}
