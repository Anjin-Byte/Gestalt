//! Chunk management system for voxel worlds.
//!
//! This module provides a complete chunk management system including:
//! - [`ChunkCoord`]: Chunk-space coordinates with neighbor calculation
//! - [`Chunk`]: Voxel storage wrapping [`BinaryChunk`](crate::core::BinaryChunk)
//! - [`ChunkState`]: Lifecycle state machine for mesh management
//! - [`DirtyTracker`]: Deduped tracking of chunks needing rebuild
//! - [`RebuildQueue`]: Priority-ordered rebuild scheduling
//! - [`ChunkManager`]: Central orchestrator for all chunk operations
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        ChunkManager                             │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  HashMap<ChunkCoord, Chunk>   - Chunk storage                   │
//! │  DirtyTracker                 - Tracks dirty chunks             │
//! │  RebuildQueue                 - Priority scheduling             │
//! │  RebuildConfig                - Budget configuration            │
//! └─────────────────────────────────────────────────────────────────┘
//!                           │
//!              ┌────────────┼────────────┐
//!              ▼            ▼            ▼
//!         ┌────────┐  ┌────────┐   ┌────────┐
//!         │ Chunk  │  │ Chunk  │   │ Chunk  │
//!         ├────────┤  ├────────┤   ├────────┤
//!         │coord   │  │coord   │   │coord   │
//!         │state   │  │state   │   │state   │
//!         │version │  │version │   │version │
//!         │voxels  │  │voxels  │   │voxels  │
//!         │mesh    │  │mesh    │   │mesh    │
//!         └────────┘  └────────┘   └────────┘
//! ```
//!
//! # State Machine
//!
//! Each chunk's mesh goes through the following states:
//!
//! ```text
//!                     ┌──────────────────────────────────┐
//!                     │                                  │
//!                     ▼                                  │
//! ┌─────────┐    ┌─────────┐    ┌──────────┐    ┌──────────────┐
//! │  Clean  │───▶│  Dirty  │───▶│  Meshing │───▶│ ReadyToSwap  │
//! └─────────┘    └─────────┘    └──────────┘    └──────────────┘
//!      ▲                                              │
//!      │                                              │
//!      └──────────────────────────────────────────────┘
//!                    (swap complete)
//! ```
//!
//! # Usage
//!
//! ```
//! use greedy_mesher::chunk::{ChunkManager, RebuildConfig};
//!
//! // Create manager with default config
//! let mut manager = ChunkManager::new();
//!
//! // Set voxels (automatically marks chunks dirty)
//! manager.set_voxel([10.0, 10.0, 10.0], 1);
//! manager.set_voxel([11.0, 10.0, 10.0], 1);
//!
//! // Run frame update (rebuilds within budget, swaps meshes)
//! let camera_pos = [0.0, 0.0, 0.0];
//! let stats = manager.update(camera_pos);
//!
//! println!("Rebuilt {} chunks", stats.rebuild.chunks_rebuilt);
//! ```

pub mod coord;
pub mod state;
pub mod chunk;
pub mod dirty;
pub mod queue;
pub mod stats;
pub mod lru;
pub mod budget;
pub mod manager;
pub mod palette_repack;
pub mod palette_materials;

// Re-export primary types
pub use coord::ChunkCoord;
pub use state::{ChunkState, BoundaryFlags};
pub use chunk::{Chunk, ChunkMesh};
pub use dirty::DirtyTracker;
pub use queue::{RebuildQueue, RebuildRequest, calculate_priority};
pub use stats::{RebuildStats, SwapStats, FrameStats, ChunkDebugInfo, RebuildConfig};
pub use lru::LruTracker;
pub use budget::{MemoryBudget, EvictionCandidate, EvictionStats};
pub use manager::ChunkManager;
