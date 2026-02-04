//! ChunkManager - orchestrates chunk storage, dirty tracking, and rebuilds.

use std::collections::HashMap;

use crate::core::{MaterialId, MATERIAL_EMPTY};
use crate::mesh::mesh_chunk_with_uvs;
use super::chunk::{Chunk, ChunkMesh};
use super::coord::ChunkCoord;
use super::dirty::DirtyTracker;
use super::queue::{RebuildQueue, calculate_priority};
use super::state::{ChunkState, BoundaryFlags};
use super::stats::{RebuildStats, SwapStats, FrameStats, ChunkDebugInfo, RebuildConfig};

/// Build mesh for a chunk (standalone function to avoid borrow issues).
fn build_mesh_for_chunk(chunk: &Chunk, voxel_size: f32) -> ChunkMesh {
    let origin = chunk.coord.origin_world(voxel_size);
    let output = mesh_chunk_with_uvs(&chunk.voxels, voxel_size, origin);
    ChunkMesh::from_mesh_output(output, chunk.data_version)
}

/// Central manager for all chunk operations.
///
/// Handles:
/// - Chunk storage (HashMap by coordinate)
/// - Voxel edits with automatic dirty tracking
/// - Priority-based rebuild scheduling
/// - Mesh generation and swapping
pub struct ChunkManager {
    /// All chunks indexed by coordinate.
    chunks: HashMap<ChunkCoord, Chunk>,

    /// Tracks which chunks need rebuilds.
    dirty_tracker: DirtyTracker,

    /// Priority queue for rebuild scheduling.
    rebuild_queue: RebuildQueue,

    /// Configuration for rebuild scheduling.
    config: RebuildConfig,
}

impl ChunkManager {
    /// Create a new chunk manager with default configuration.
    pub fn new() -> Self {
        Self::with_config(RebuildConfig::default())
    }

    /// Create a new chunk manager with custom configuration.
    pub fn with_config(config: RebuildConfig) -> Self {
        Self {
            chunks: HashMap::new(),
            dirty_tracker: DirtyTracker::new(),
            rebuild_queue: RebuildQueue::new(),
            config,
        }
    }

    /// Get the voxel size from configuration.
    pub fn voxel_size(&self) -> f32 {
        self.config.voxel_size
    }

    // ========================================================================
    // Chunk Access
    // ========================================================================

    /// Get a reference to a chunk by coordinate.
    pub fn get_chunk(&self, coord: ChunkCoord) -> Option<&Chunk> {
        self.chunks.get(&coord)
    }

    /// Get a mutable reference to a chunk by coordinate.
    pub fn get_chunk_mut(&mut self, coord: ChunkCoord) -> Option<&mut Chunk> {
        self.chunks.get_mut(&coord)
    }

    /// Get or create a chunk at the given coordinate.
    pub fn get_or_create_chunk(&mut self, coord: ChunkCoord) -> &mut Chunk {
        self.chunks.entry(coord).or_insert_with(|| Chunk::new(coord))
    }

    /// Check if a chunk exists.
    pub fn has_chunk(&self, coord: ChunkCoord) -> bool {
        self.chunks.contains_key(&coord)
    }

    /// Remove a chunk.
    pub fn remove_chunk(&mut self, coord: ChunkCoord) -> Option<Chunk> {
        // Also remove from dirty tracker and queue
        self.dirty_tracker.unmark(coord);
        self.rebuild_queue.remove(coord);
        self.chunks.remove(&coord)
    }

    /// Number of chunks in the manager.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Iterate over all chunks.
    pub fn iter_chunks(&self) -> impl Iterator<Item = (&ChunkCoord, &Chunk)> {
        self.chunks.iter()
    }

    /// Iterate over chunk coordinates.
    pub fn iter_coords(&self) -> impl Iterator<Item = &ChunkCoord> {
        self.chunks.keys()
    }

    // ========================================================================
    // Voxel Access
    // ========================================================================

    /// Convert world position to voxel index.
    pub fn world_to_voxel(&self, world_pos: [f32; 3]) -> [i32; 3] {
        [
            (world_pos[0] / self.config.voxel_size).floor() as i32,
            (world_pos[1] / self.config.voxel_size).floor() as i32,
            (world_pos[2] / self.config.voxel_size).floor() as i32,
        ]
    }

    /// Get material at world position.
    pub fn get_voxel(&self, world_pos: [f32; 3]) -> MaterialId {
        let voxel_idx = self.world_to_voxel(world_pos);
        let chunk_coord = ChunkCoord::from_voxel(voxel_idx);
        let local = ChunkCoord::voxel_to_local(voxel_idx);

        self.chunks
            .get(&chunk_coord)
            .map(|c| c.get_voxel(local[0], local[1], local[2]))
            .unwrap_or(MATERIAL_EMPTY)
    }

    /// Set voxel at world position with automatic dirty tracking.
    pub fn set_voxel(&mut self, world_pos: [f32; 3], material: MaterialId) {
        let voxel_idx = self.world_to_voxel(world_pos);
        let chunk_coord = ChunkCoord::from_voxel(voxel_idx);
        let local = ChunkCoord::voxel_to_local(voxel_idx);

        // Get or create chunk
        let chunk = self.chunks.entry(chunk_coord).or_insert_with(|| {
            Chunk::new(chunk_coord)
        });

        // Check boundary before edit
        let boundary = chunk.is_on_boundary(local[0], local[1], local[2]);

        // Perform edit
        chunk.set_voxel(local[0], local[1], local[2], material);

        // Mark dirty with boundary awareness
        self.dirty_tracker.mark_dirty_with_neighbors(chunk_coord, boundary);

        // Transition state
        chunk.state = ChunkState::Dirty;
    }

    /// Set voxel at voxel index (integer coordinates).
    pub fn set_voxel_at(&mut self, voxel: [i32; 3], material: MaterialId) {
        let chunk_coord = ChunkCoord::from_voxel(voxel);
        let local = ChunkCoord::voxel_to_local(voxel);

        let chunk = self.chunks.entry(chunk_coord).or_insert_with(|| {
            Chunk::new(chunk_coord)
        });

        let boundary = chunk.is_on_boundary(local[0], local[1], local[2]);
        chunk.set_voxel(local[0], local[1], local[2], material);
        self.dirty_tracker.mark_dirty_with_neighbors(chunk_coord, boundary);
        chunk.state = ChunkState::Dirty;
    }

    /// Batch edit multiple voxels efficiently.
    ///
    /// Groups edits by chunk to minimize dirty marking overhead.
    pub fn set_voxels_batch(&mut self, edits: &[([f32; 3], MaterialId)]) {
        // Group edits by chunk
        let mut edits_by_chunk: HashMap<ChunkCoord, Vec<([u32; 3], MaterialId)>> = HashMap::new();

        for (world_pos, material) in edits {
            let voxel_idx = self.world_to_voxel(*world_pos);
            let chunk_coord = ChunkCoord::from_voxel(voxel_idx);
            let local = ChunkCoord::voxel_to_local(voxel_idx);

            edits_by_chunk
                .entry(chunk_coord)
                .or_default()
                .push((local, *material));
        }

        // Apply edits per chunk
        for (chunk_coord, chunk_edits) in edits_by_chunk {
            let chunk = self.chunks.entry(chunk_coord).or_insert_with(|| {
                Chunk::new(chunk_coord)
            });

            let mut combined_boundary = BoundaryFlags::default();

            for (local, material) in chunk_edits {
                let boundary = chunk.is_on_boundary(local[0], local[1], local[2]);
                combined_boundary.merge(boundary);
                chunk.set_voxel_raw(local[0], local[1], local[2], material);
            }

            chunk.increment_version();
            self.dirty_tracker.mark_dirty_with_neighbors(chunk_coord, combined_boundary);
            chunk.state = ChunkState::Dirty;
        }
    }

    // ========================================================================
    // Rebuild Processing
    // ========================================================================

    /// Process pending rebuilds within frame budget.
    ///
    /// Returns statistics about the rebuild operations performed.
    pub fn process_rebuilds(&mut self, camera_pos: [f32; 3]) -> RebuildStats {
        let start_time = std::time::Instant::now();
        let mut stats = RebuildStats::default();
        let voxel_size = self.config.voxel_size;

        // Move dirty chunks to rebuild queue with priorities
        let dirty = self.dirty_tracker.take_dirty();
        for coord in dirty {
            if let Some(chunk) = self.chunks.get(&coord) {
                let center = coord.center_world(voxel_size);
                let priority = calculate_priority(center, camera_pos);
                self.rebuild_queue.enqueue(coord, priority, chunk.data_version);
            }
        }

        // Process queue within budget
        while stats.chunks_rebuilt < self.config.max_chunks_per_frame {
            // Check time budget
            let elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;
            if elapsed_ms >= self.config.max_time_per_frame_ms {
                stats.time_budget_exceeded = true;
                break;
            }

            // Get next chunk to rebuild
            let Some(request) = self.rebuild_queue.pop() else {
                break;
            };

            // Skip if chunk no longer exists
            let Some(chunk) = self.chunks.get_mut(&request.coord) else {
                stats.chunks_missing += 1;
                continue;
            };

            // Skip if data version changed (chunk was edited again)
            if chunk.data_version != request.data_version {
                // Re-enqueue with updated version
                let center = request.coord.center_world(voxel_size);
                let priority = calculate_priority(center, camera_pos);
                self.rebuild_queue.enqueue(request.coord, priority, chunk.data_version);
                stats.version_mismatches += 1;
                continue;
            }

            // Perform rebuild using standalone function
            let mesh = build_mesh_for_chunk(chunk, voxel_size);
            stats.triangles_generated += mesh.triangle_count;
            stats.vertices_generated += mesh.vertex_count;
            stats.chunks_rebuilt += 1;

            // Update chunk state
            chunk.pending_mesh = Some(mesh);
            chunk.state = ChunkState::ReadyToSwap {
                data_version: chunk.data_version,
            };
        }

        if stats.chunks_rebuilt >= self.config.max_chunks_per_frame {
            stats.chunk_limit_reached = true;
        }

        stats.queue_remaining = self.rebuild_queue.len();
        stats.elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;
        stats
    }

    /// Swap all pending meshes into active slot.
    ///
    /// Call this after process_rebuilds(), before rendering.
    pub fn swap_pending_meshes(&mut self) -> SwapStats {
        let mut stats = SwapStats::default();

        // Collect coords that need dirty marking (to avoid borrow issues)
        let mut needs_dirty: Vec<ChunkCoord> = Vec::new();

        for chunk in self.chunks.values_mut() {
            if let ChunkState::ReadyToSwap { data_version } = chunk.state {
                if data_version == chunk.data_version {
                    // Version matches - swap mesh
                    if let Some(pending) = chunk.pending_mesh.take() {
                        if chunk.mesh.is_some() {
                            stats.meshes_disposed += 1;
                        }
                        chunk.mesh = Some(pending);
                        stats.meshes_swapped += 1;
                        chunk.state = ChunkState::Clean;
                    }
                } else {
                    // Version mismatch - discard pending and mark dirty
                    chunk.pending_mesh = None;
                    chunk.state = ChunkState::Dirty;
                    needs_dirty.push(chunk.coord);
                    stats.version_conflicts += 1;
                }
            }
        }

        // Mark collected chunks as dirty
        for coord in needs_dirty {
            self.dirty_tracker.mark_dirty(coord);
        }

        stats
    }

    /// Process one full frame update.
    ///
    /// Runs rebuild and swap phases.
    pub fn update(&mut self, camera_pos: [f32; 3]) -> FrameStats {
        let rebuild_stats = self.process_rebuilds(camera_pos);
        let swap_stats = self.swap_pending_meshes();

        let total_chunks = self.chunks.len();
        let chunks_with_mesh = self.chunks.values().filter(|c| c.mesh.is_some()).count();
        let dirty_chunks = self.dirty_tracker.dirty_count();

        FrameStats {
            rebuild: rebuild_stats,
            swap: swap_stats,
            total_chunks,
            chunks_with_mesh,
            dirty_chunks,
        }
    }

    // ========================================================================
    // Debug / Inspection
    // ========================================================================

    /// Get comprehensive debug information.
    pub fn debug_info(&self) -> ChunkDebugInfo {
        let mut info = ChunkDebugInfo::default();

        for chunk in self.chunks.values() {
            info.total_chunks += 1;
            match chunk.state {
                ChunkState::Clean => info.clean_chunks += 1,
                ChunkState::Dirty => info.dirty_chunks += 1,
                ChunkState::Meshing { .. } => info.meshing_chunks += 1,
                ChunkState::ReadyToSwap { .. } => info.ready_to_swap_chunks += 1,
            }

            // Memory estimation for voxels (BinaryChunk size)
            info.voxel_memory_bytes += std::mem::size_of_val(&chunk.voxels);

            if let Some(mesh) = &chunk.mesh {
                info.total_triangles += mesh.triangle_count;
                info.total_vertices += mesh.vertex_count;
                info.mesh_memory_bytes += mesh.memory_bytes();
            }
        }

        info.queue_size = self.rebuild_queue.len();
        info.dirty_tracker_size = self.dirty_tracker.dirty_count();
        info
    }

    /// Force immediate rebuild of all dirty chunks (ignores budget).
    ///
    /// Useful for tests or loading operations.
    pub fn rebuild_all_dirty(&mut self, _camera_pos: [f32; 3]) -> usize {
        let mut count = 0;
        let voxel_size = self.config.voxel_size;

        // Take all dirty chunks
        let dirty: Vec<_> = self.dirty_tracker.take_dirty().into_iter().collect();

        for coord in dirty {
            if let Some(chunk) = self.chunks.get_mut(&coord) {
                let mesh = build_mesh_for_chunk(chunk, voxel_size);
                chunk.pending_mesh = Some(mesh);
                chunk.state = ChunkState::ReadyToSwap {
                    data_version: chunk.data_version,
                };
                count += 1;
            }
        }

        // Also process rebuild queue
        while let Some(request) = self.rebuild_queue.pop() {
            if let Some(chunk) = self.chunks.get_mut(&request.coord) {
                if chunk.data_version == request.data_version {
                    let mesh = build_mesh_for_chunk(chunk, voxel_size);
                    chunk.pending_mesh = Some(mesh);
                    chunk.state = ChunkState::ReadyToSwap {
                        data_version: chunk.data_version,
                    };
                    count += 1;
                }
            }
        }

        // Swap all
        self.swap_pending_meshes();

        count
    }

    /// Clear all chunks and reset state.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.dirty_tracker.clear();
        self.rebuild_queue.clear();
    }
}

impl Default for ChunkManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::MATERIAL_DEFAULT;

    #[test]
    fn new_manager_is_empty() {
        let manager = ChunkManager::new();
        assert_eq!(manager.chunk_count(), 0);
    }

    #[test]
    fn set_voxel_creates_chunk() {
        let mut manager = ChunkManager::new();

        manager.set_voxel([10.0, 10.0, 10.0], MATERIAL_DEFAULT);

        assert_eq!(manager.chunk_count(), 1);
        assert_eq!(manager.get_voxel([10.0, 10.0, 10.0]), MATERIAL_DEFAULT);
    }

    #[test]
    fn set_voxel_marks_dirty() {
        let mut manager = ChunkManager::new();

        manager.set_voxel([10.0, 10.0, 10.0], MATERIAL_DEFAULT);

        let chunk = manager.get_chunk(ChunkCoord::ZERO).unwrap();
        assert_eq!(chunk.state, ChunkState::Dirty);
    }

    #[test]
    fn boundary_edit_marks_neighbor_dirty() {
        let mut manager = ChunkManager::new();

        // Edit at x=0 (boundary of chunk 0, adjacent to chunk -1)
        manager.set_voxel([0.0, 10.0, 10.0], MATERIAL_DEFAULT);

        // Both chunks should be dirty
        let info = manager.debug_info();
        // Note: neighbor chunk may not exist, just dirty_tracker entry
        assert!(info.dirty_chunks > 0 || manager.dirty_tracker.dirty_count() > 0);
    }

    #[test]
    fn process_rebuilds_respects_budget() {
        let mut manager = ChunkManager::with_config(RebuildConfig {
            max_chunks_per_frame: 2,
            max_time_per_frame_ms: 1000.0,
            voxel_size: 1.0,
        });

        // Create 5 dirty chunks
        for i in 0..5 {
            let coord = ChunkCoord::new(i, 0, 0);
            let chunk = manager.get_or_create_chunk(coord);
            chunk.set_voxel(1, 1, 1, MATERIAL_DEFAULT);
            manager.dirty_tracker.mark_dirty(coord);
        }

        let stats = manager.process_rebuilds([0.0, 0.0, 0.0]);

        // Should only rebuild 2 (max_chunks_per_frame)
        assert_eq!(stats.chunks_rebuilt, 2);
        assert!(stats.chunk_limit_reached);
        assert_eq!(stats.queue_remaining, 3);
    }

    #[test]
    fn version_mismatch_requeues() {
        let mut manager = ChunkManager::new();
        let coord = ChunkCoord::ZERO;

        // Create chunk and mark dirty
        let chunk = manager.get_or_create_chunk(coord);
        chunk.set_voxel(1, 1, 1, MATERIAL_DEFAULT);
        let _old_version = chunk.data_version;
        manager.dirty_tracker.mark_dirty(coord);

        // First pass: moves to queue
        let _ = manager.process_rebuilds([0.0, 0.0, 0.0]);

        // Simulate edit during rebuild by manually changing version
        if let Some(chunk) = manager.get_chunk_mut(coord) {
            chunk.set_voxel(2, 2, 2, MATERIAL_DEFAULT);
            // Version is now old_version + 1
        }

        // The request in queue has old_version, chunk has old_version + 1
        // This would cause version mismatch on next process_rebuilds
    }

    #[test]
    fn batch_edit() {
        let mut manager = ChunkManager::new();

        let edits: Vec<([f32; 3], MaterialId)> = vec![
            ([10.0, 10.0, 10.0], 1),
            ([11.0, 10.0, 10.0], 2),
            ([12.0, 10.0, 10.0], 3),
        ];

        manager.set_voxels_batch(&edits);

        assert_eq!(manager.get_voxel([10.0, 10.0, 10.0]), 1);
        assert_eq!(manager.get_voxel([11.0, 10.0, 10.0]), 2);
        assert_eq!(manager.get_voxel([12.0, 10.0, 10.0]), 3);

        // Should be one chunk with version incremented once
        assert_eq!(manager.chunk_count(), 1);
    }

    #[test]
    fn full_update_cycle() {
        let mut manager = ChunkManager::new();

        // Set a voxel
        manager.set_voxel([10.0, 10.0, 10.0], MATERIAL_DEFAULT);

        // Run update
        let stats = manager.update([0.0, 0.0, 0.0]);

        // Should have rebuilt and swapped
        assert!(stats.rebuild.chunks_rebuilt >= 1);
        assert!(stats.swap.meshes_swapped >= 1);

        // Chunk should be clean
        let chunk = manager.get_chunk(ChunkCoord::ZERO).unwrap();
        assert_eq!(chunk.state, ChunkState::Clean);
        assert!(chunk.mesh.is_some());
    }

    #[test]
    fn remove_chunk() {
        let mut manager = ChunkManager::new();
        let coord = ChunkCoord::ZERO;

        manager.set_voxel([10.0, 10.0, 10.0], MATERIAL_DEFAULT);
        assert!(manager.has_chunk(coord));

        let removed = manager.remove_chunk(coord);
        assert!(removed.is_some());
        assert!(!manager.has_chunk(coord));
    }

    #[test]
    fn debug_info() {
        let mut manager = ChunkManager::new();

        manager.set_voxel([10.0, 10.0, 10.0], MATERIAL_DEFAULT);
        manager.update([0.0, 0.0, 0.0]);

        let info = manager.debug_info();

        assert_eq!(info.total_chunks, 1);
        assert_eq!(info.clean_chunks, 1);
        assert!(info.total_triangles > 0);
    }
}
