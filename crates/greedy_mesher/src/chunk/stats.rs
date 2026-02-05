//! Statistics structs for chunk management operations.

/// Statistics from a single frame's rebuild operations.
#[derive(Clone, Debug, Default)]
pub struct RebuildStats {
    /// Number of chunks rebuilt this frame.
    pub chunks_rebuilt: usize,
    /// Total triangles generated across all rebuilt chunks.
    pub triangles_generated: usize,
    /// Total vertices generated across all rebuilt chunks.
    pub vertices_generated: usize,
    /// Number of requests skipped due to version mismatch.
    pub version_mismatches: usize,
    /// Number of requests skipped because chunk no longer exists.
    pub chunks_missing: usize,
    /// Number of chunks remaining in the queue.
    pub queue_remaining: usize,
    /// Whether the time budget was exceeded.
    pub time_budget_exceeded: bool,
    /// Whether the chunk count limit was reached.
    pub chunk_limit_reached: bool,
    /// Total time spent rebuilding (milliseconds).
    pub elapsed_ms: f64,
}

impl RebuildStats {
    /// Check if any rebuilds occurred.
    pub fn any_rebuilt(&self) -> bool {
        self.chunks_rebuilt > 0
    }

    /// Check if more work remains.
    pub fn has_remaining(&self) -> bool {
        self.queue_remaining > 0
    }
}

/// Statistics from mesh swap operations.
#[derive(Clone, Debug, Default)]
pub struct SwapStats {
    /// Number of meshes successfully swapped.
    pub meshes_swapped: usize,
    /// Number of old meshes disposed.
    pub meshes_disposed: usize,
    /// Number of swaps rejected due to version conflict.
    pub version_conflicts: usize,
}

impl SwapStats {
    /// Check if any swaps occurred.
    pub fn any_swapped(&self) -> bool {
        self.meshes_swapped > 0
    }
}

/// Combined frame statistics.
#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    /// Statistics from rebuild phase.
    pub rebuild: RebuildStats,
    /// Statistics from swap phase.
    pub swap: SwapStats,
    /// Statistics from eviction phase.
    pub eviction: super::budget::EvictionStats,
    /// Total chunks managed.
    pub total_chunks: usize,
    /// Chunks currently with valid meshes.
    pub chunks_with_mesh: usize,
    /// Chunks currently dirty (need rebuild).
    pub dirty_chunks: usize,
}

/// Debug information about chunk system state.
#[derive(Clone, Debug, Default)]
pub struct ChunkDebugInfo {
    /// Total number of chunks in the manager.
    pub total_chunks: usize,
    /// Chunks in Clean state.
    pub clean_chunks: usize,
    /// Chunks in Dirty state.
    pub dirty_chunks: usize,
    /// Chunks in Meshing state.
    pub meshing_chunks: usize,
    /// Chunks in ReadyToSwap state.
    pub ready_to_swap_chunks: usize,
    /// Size of the rebuild queue.
    pub queue_size: usize,
    /// Size of the dirty tracker set.
    pub dirty_tracker_size: usize,
    /// Total triangles across all meshes.
    pub total_triangles: usize,
    /// Total vertices across all meshes.
    pub total_vertices: usize,
    /// Estimated memory usage for voxel data (bytes).
    pub voxel_memory_bytes: usize,
    /// Estimated memory usage for mesh data (bytes).
    pub mesh_memory_bytes: usize,
    /// Memory budget limit (bytes).
    pub budget_max_bytes: usize,
    /// Memory usage as percentage of budget.
    pub budget_usage_percent: f32,
    /// Whether the memory budget is exceeded.
    pub budget_exceeded: bool,
}

impl ChunkDebugInfo {
    /// Get total estimated memory usage.
    pub fn total_memory_bytes(&self) -> usize {
        self.voxel_memory_bytes + self.mesh_memory_bytes
    }

    /// Get memory usage in megabytes.
    pub fn total_memory_mb(&self) -> f32 {
        self.total_memory_bytes() as f32 / (1024.0 * 1024.0)
    }
}

/// Configuration for rebuild scheduling.
#[derive(Clone, Debug)]
pub struct RebuildConfig {
    /// Maximum chunks to rebuild per frame.
    pub max_chunks_per_frame: usize,

    /// Maximum time (ms) to spend rebuilding per frame.
    pub max_time_per_frame_ms: f64,

    /// Default voxel size for world coordinate calculations.
    pub voxel_size: f32,
}

impl Default for RebuildConfig {
    fn default() -> Self {
        Self {
            max_chunks_per_frame: 4,
            max_time_per_frame_ms: 8.0, // ~half a frame at 60fps
            voxel_size: 1.0,
        }
    }
}

impl RebuildConfig {
    /// Create a config optimized for high-end systems.
    pub fn high_performance() -> Self {
        Self {
            max_chunks_per_frame: 8,
            max_time_per_frame_ms: 12.0,
            voxel_size: 1.0,
        }
    }

    /// Create a config optimized for low-end systems.
    pub fn low_performance() -> Self {
        Self {
            max_chunks_per_frame: 2,
            max_time_per_frame_ms: 4.0,
            voxel_size: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_stats_default() {
        let stats = RebuildStats::default();
        assert!(!stats.any_rebuilt());
        assert!(!stats.has_remaining());
    }

    #[test]
    fn rebuild_stats_any_rebuilt() {
        let mut stats = RebuildStats::default();
        stats.chunks_rebuilt = 1;
        assert!(stats.any_rebuilt());
    }

    #[test]
    fn rebuild_stats_has_remaining() {
        let mut stats = RebuildStats::default();
        stats.queue_remaining = 5;
        assert!(stats.has_remaining());
    }

    #[test]
    fn swap_stats_default() {
        let stats = SwapStats::default();
        assert!(!stats.any_swapped());
    }

    #[test]
    fn chunk_debug_info_memory() {
        let mut info = ChunkDebugInfo::default();
        info.voxel_memory_bytes = 1024 * 1024; // 1 MB
        info.mesh_memory_bytes = 512 * 1024; // 0.5 MB

        assert_eq!(info.total_memory_bytes(), 1536 * 1024);
        assert!((info.total_memory_mb() - 1.5).abs() < 0.01);
    }

    #[test]
    fn rebuild_config_default() {
        let config = RebuildConfig::default();
        assert_eq!(config.max_chunks_per_frame, 4);
        assert_eq!(config.max_time_per_frame_ms, 8.0);
    }

    #[test]
    fn rebuild_config_variants() {
        let high = RebuildConfig::high_performance();
        let low = RebuildConfig::low_performance();

        assert!(high.max_chunks_per_frame > low.max_chunks_per_frame);
        assert!(high.max_time_per_frame_ms > low.max_time_per_frame_ms);
    }
}
