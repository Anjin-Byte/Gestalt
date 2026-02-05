//! Memory budget configuration and eviction types.
//!
//! Defines the budget limits, hysteresis watermarks, and eviction
//! result types used by [`ChunkManager`](super::manager::ChunkManager).

use super::coord::ChunkCoord;

/// Memory budget configuration with hysteresis watermarks.
///
/// Eviction starts when usage exceeds the high watermark and stops
/// when usage drops below the low watermark. This prevents thrashing
/// at the budget boundary.
#[derive(Clone, Debug)]
pub struct MemoryBudget {
    /// Maximum total memory in bytes.
    pub max_bytes: usize,

    /// High watermark as a fraction of `max_bytes` (0.0–1.0).
    /// Eviction starts when usage exceeds this threshold.
    pub high_watermark: f32,

    /// Low watermark as a fraction of `max_bytes` (0.0–1.0).
    /// Eviction stops when usage drops below this threshold.
    pub low_watermark: f32,

    /// Minimum number of chunks to keep (never evict below this).
    pub min_chunks: usize,
}

impl Default for MemoryBudget {
    fn default() -> Self {
        Self {
            max_bytes: 512 * 1024 * 1024, // 512 MB
            high_watermark: 0.90,
            low_watermark: 0.75,
            min_chunks: 8,
        }
    }
}

impl MemoryBudget {
    /// Budget for low-memory systems (128 MB).
    pub fn low_memory() -> Self {
        Self {
            max_bytes: 128 * 1024 * 1024,
            high_watermark: 0.85,
            low_watermark: 0.70,
            min_chunks: 4,
        }
    }

    /// Budget for high-memory systems (1 GB).
    pub fn high_memory() -> Self {
        Self {
            max_bytes: 1024 * 1024 * 1024,
            high_watermark: 0.90,
            low_watermark: 0.75,
            min_chunks: 16,
        }
    }

    /// High watermark in bytes.
    pub fn high_watermark_bytes(&self) -> usize {
        (self.max_bytes as f64 * self.high_watermark as f64).round() as usize
    }

    /// Low watermark in bytes (eviction target).
    pub fn low_watermark_bytes(&self) -> usize {
        (self.max_bytes as f64 * self.low_watermark as f64).round() as usize
    }

    /// Whether memory usage exceeds the high watermark.
    pub fn is_exceeded(&self, current_bytes: usize) -> bool {
        current_bytes > self.high_watermark_bytes()
    }

    /// Whether memory usage is below the low watermark.
    pub fn is_satisfied(&self, current_bytes: usize) -> bool {
        current_bytes <= self.low_watermark_bytes()
    }
}

/// A chunk that is a candidate for eviction.
#[derive(Clone, Debug)]
pub struct EvictionCandidate {
    /// Chunk coordinate.
    pub coord: ChunkCoord,
    /// Eviction priority (lower = evict sooner).
    pub priority: f32,
    /// Estimated memory used by this chunk.
    pub memory_bytes: usize,
}

/// Statistics from an eviction operation.
#[derive(Clone, Debug, Default)]
pub struct EvictionStats {
    /// Number of chunks evicted.
    pub chunks_evicted: usize,
    /// Total bytes freed.
    pub bytes_freed: usize,
    /// Number of chunks skipped (Dirty/Meshing).
    pub chunks_skipped: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget() {
        let budget = MemoryBudget::default();
        assert_eq!(budget.max_bytes, 512 * 1024 * 1024);
        assert_eq!(budget.high_watermark, 0.90);
        assert_eq!(budget.low_watermark, 0.75);
        assert_eq!(budget.min_chunks, 8);
    }

    #[test]
    fn watermark_bytes() {
        let budget = MemoryBudget {
            max_bytes: 1000,
            high_watermark: 0.90,
            low_watermark: 0.75,
            min_chunks: 1,
        };
        assert_eq!(budget.high_watermark_bytes(), 900);
        assert_eq!(budget.low_watermark_bytes(), 750);
    }

    #[test]
    fn is_exceeded() {
        let budget = MemoryBudget {
            max_bytes: 1000,
            high_watermark: 0.90,
            low_watermark: 0.75,
            min_chunks: 1,
        };
        assert!(!budget.is_exceeded(899));
        assert!(!budget.is_exceeded(900));
        assert!(budget.is_exceeded(901));
    }

    #[test]
    fn is_satisfied() {
        let budget = MemoryBudget {
            max_bytes: 1000,
            high_watermark: 0.90,
            low_watermark: 0.75,
            min_chunks: 1,
        };
        assert!(budget.is_satisfied(749));
        assert!(budget.is_satisfied(750));
        assert!(!budget.is_satisfied(751));
    }

    #[test]
    fn hysteresis_gap() {
        let budget = MemoryBudget::default();
        // There should be a gap between watermarks
        assert!(budget.high_watermark_bytes() > budget.low_watermark_bytes());
    }

    #[test]
    fn presets_differ() {
        let low = MemoryBudget::low_memory();
        let high = MemoryBudget::high_memory();
        assert!(high.max_bytes > low.max_bytes);
    }

    #[test]
    fn eviction_stats_default() {
        let stats = EvictionStats::default();
        assert_eq!(stats.chunks_evicted, 0);
        assert_eq!(stats.bytes_freed, 0);
        assert_eq!(stats.chunks_skipped, 0);
    }
}
