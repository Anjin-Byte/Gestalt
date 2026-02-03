# Chunk Management System

Technical specification for the voxel chunk management system including dirty tracking, rebuild scheduling, and state management.

## 1. Chunk Data Structure

### 1.1 Chunk Identifier

```rust
/// Chunk coordinate in chunk-space (not world-space)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const ZERO: ChunkCoord = ChunkCoord { x: 0, y: 0, z: 0 };

    /// Get the 6 face-adjacent neighbors
    pub fn neighbors(&self) -> [ChunkCoord; 6] {
        [
            ChunkCoord { x: self.x + 1, y: self.y, z: self.z },
            ChunkCoord { x: self.x - 1, y: self.y, z: self.z },
            ChunkCoord { x: self.x, y: self.y + 1, z: self.z },
            ChunkCoord { x: self.x, y: self.y - 1, z: self.z },
            ChunkCoord { x: self.x, y: self.y, z: self.z + 1 },
            ChunkCoord { x: self.x, y: self.y, z: self.z - 1 },
        ]
    }

    /// Convert world position to chunk coordinate
    pub fn from_world(world_pos: [f32; 3], chunk_size: u32, voxel_size: f32) -> Self {
        let chunk_world_size = chunk_size as f32 * voxel_size;
        ChunkCoord {
            x: (world_pos[0] / chunk_world_size).floor() as i32,
            y: (world_pos[1] / chunk_world_size).floor() as i32,
            z: (world_pos[2] / chunk_world_size).floor() as i32,
        }
    }

    /// Convert voxel index to chunk coordinate
    pub fn from_voxel(voxel: [i32; 3], chunk_size: u32) -> Self {
        let cs = chunk_size as i32;
        ChunkCoord {
            x: voxel[0].div_euclid(cs),
            y: voxel[1].div_euclid(cs),
            z: voxel[2].div_euclid(cs),
        }
    }
}
```

### 1.2 Chunk State Machine

```rust
/// Lifecycle state of a chunk's mesh
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkState {
    /// Mesh is up-to-date with voxel data
    Clean,

    /// Voxel data changed; mesh needs rebuild
    Dirty,

    /// Currently being meshed (async job in progress)
    Meshing {
        /// Version of voxel data when meshing started
        data_version: u64,
    },

    /// New mesh ready; waiting to swap into render
    ReadyToSwap {
        /// Version of voxel data the mesh was built from
        data_version: u64,
    },
}
```

**State Transitions:**

```
                    ┌──────────────────────────────────┐
                    │                                  │
                    ▼                                  │
┌─────────┐    ┌─────────┐    ┌──────────┐    ┌──────────────┐
│  Clean  │───▶│  Dirty  │───▶│  Meshing │───▶│ ReadyToSwap  │
└─────────┘    └─────────┘    └──────────┘    └──────────────┘
     ▲              │              │                  │
     │              │              │                  │
     │              ▼              ▼                  │
     │         (edit during   (edit during           │
     │          meshing)       meshing)              │
     │              │              │                  │
     │              └──────┬───────┘                  │
     │                     │                         │
     │                     ▼                         │
     │              ┌─────────┐                      │
     │              │  Dirty  │◀─────────────────────┘
     │              └─────────┘   (version mismatch)
     │                     │
     └─────────────────────┘
           (swap complete)
```

### 1.3 Chunk Data

```rust
/// Complete chunk data structure
pub struct Chunk {
    pub coord: ChunkCoord,
    pub state: ChunkState,

    /// Monotonically increasing version, incremented on any voxel edit
    pub data_version: u64,

    /// Voxel storage (dense array for simplicity)
    pub voxels: Vec<Voxel>,

    /// Cached mesh data (if state is Clean or ReadyToSwap)
    pub mesh: Option<ChunkMesh>,

    /// Pending mesh from async job (if state is ReadyToSwap)
    pub pending_mesh: Option<ChunkMesh>,
}

/// Mesh data for a single chunk
pub struct ChunkMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub colors: Option<Vec<f32>>,

    /// Version of voxel data this mesh was built from
    pub data_version: u64,

    /// Statistics
    pub triangle_count: usize,
    pub vertex_count: usize,
}

impl Chunk {
    /// Chunk dimension: 64³ voxels total (62³ usable with 1-voxel padding for binary meshing)
    pub const SIZE: u32 = 64;

    pub fn new(coord: ChunkCoord) -> Self {
        Self {
            coord,
            state: ChunkState::Dirty, // New chunks need initial mesh
            data_version: 0,
            voxels: vec![Voxel::EMPTY; (Self::SIZE * Self::SIZE * Self::SIZE) as usize],
            mesh: None,
            pending_mesh: None,
        }
    }

    /// Get voxel at local coordinates (0..SIZE)
    pub fn get_voxel(&self, x: u32, y: u32, z: u32) -> Voxel {
        debug_assert!(x < Self::SIZE && y < Self::SIZE && z < Self::SIZE);
        let idx = (z * Self::SIZE * Self::SIZE + y * Self::SIZE + x) as usize;
        self.voxels[idx]
    }

    /// Set voxel at local coordinates
    pub fn set_voxel(&mut self, x: u32, y: u32, z: u32, voxel: Voxel) {
        debug_assert!(x < Self::SIZE && y < Self::SIZE && z < Self::SIZE);
        let idx = (z * Self::SIZE * Self::SIZE + y * Self::SIZE + x) as usize;
        self.voxels[idx] = voxel;
        self.data_version += 1;
    }

    /// Check if local coordinate is on chunk boundary
    pub fn is_on_boundary(&self, x: u32, y: u32, z: u32) -> BoundaryFlags {
        BoundaryFlags {
            neg_x: x == 0,
            pos_x: x == Self::SIZE - 1,
            neg_y: y == 0,
            pos_y: y == Self::SIZE - 1,
            neg_z: z == 0,
            pos_z: z == Self::SIZE - 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BoundaryFlags {
    pub neg_x: bool,
    pub pos_x: bool,
    pub neg_y: bool,
    pub pos_y: bool,
    pub neg_z: bool,
    pub pos_z: bool,
}

impl BoundaryFlags {
    pub fn any(&self) -> bool {
        self.neg_x || self.pos_x || self.neg_y || self.pos_y || self.neg_z || self.pos_z
    }

    /// Get neighbor chunk offsets that need to be marked dirty
    pub fn affected_neighbors(&self) -> Vec<[i32; 3]> {
        let mut neighbors = Vec::new();
        if self.neg_x { neighbors.push([-1, 0, 0]); }
        if self.pos_x { neighbors.push([1, 0, 0]); }
        if self.neg_y { neighbors.push([0, -1, 0]); }
        if self.pos_y { neighbors.push([0, 1, 0]); }
        if self.neg_z { neighbors.push([0, 0, -1]); }
        if self.pos_z { neighbors.push([0, 0, 1]); }
        neighbors
    }
}
```

---

## 2. Dirty Tracking

### 2.1 Dirty Set

```rust
use std::collections::HashSet;

/// Tracks which chunks need mesh rebuilds
pub struct DirtyTracker {
    /// Set of dirty chunk coordinates (deduped by nature of HashSet)
    dirty_chunks: HashSet<ChunkCoord>,
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self {
            dirty_chunks: HashSet::new(),
        }
    }

    /// Mark a single chunk as dirty
    pub fn mark_dirty(&mut self, coord: ChunkCoord) {
        self.dirty_chunks.insert(coord);
    }

    /// Mark chunk and boundary-affected neighbors as dirty
    pub fn mark_dirty_with_neighbors(
        &mut self,
        coord: ChunkCoord,
        boundary: BoundaryFlags,
    ) {
        self.dirty_chunks.insert(coord);

        for offset in boundary.affected_neighbors() {
            let neighbor = ChunkCoord {
                x: coord.x + offset[0],
                y: coord.y + offset[1],
                z: coord.z + offset[2],
            };
            self.dirty_chunks.insert(neighbor);
        }
    }

    /// Take all dirty chunks (clears the set)
    pub fn take_dirty(&mut self) -> HashSet<ChunkCoord> {
        std::mem::take(&mut self.dirty_chunks)
    }

    /// Check if any chunks are dirty
    pub fn has_dirty(&self) -> bool {
        !self.dirty_chunks.is_empty()
    }

    /// Number of dirty chunks
    pub fn dirty_count(&self) -> usize {
        self.dirty_chunks.len()
    }
}
```

### 2.2 Edit API with Automatic Dirty Marking

```rust
impl ChunkManager {
    /// Edit a voxel with automatic dirty marking
    pub fn set_voxel(&mut self, world_pos: [f32; 3], voxel: Voxel) {
        let voxel_idx = self.world_to_voxel(world_pos);
        let chunk_coord = ChunkCoord::from_voxel(voxel_idx, Chunk::SIZE);

        // Get or create chunk
        let chunk = self.chunks.entry(chunk_coord).or_insert_with(|| {
            Chunk::new(chunk_coord)
        });

        // Calculate local coordinates within chunk
        let local = self.voxel_to_local(voxel_idx);

        // Check boundary before edit
        let boundary = chunk.is_on_boundary(local[0], local[1], local[2]);

        // Perform edit
        chunk.set_voxel(local[0], local[1], local[2], voxel);

        // Mark dirty with boundary awareness
        self.dirty_tracker.mark_dirty_with_neighbors(chunk_coord, boundary);

        // Transition state
        chunk.state = ChunkState::Dirty;
    }

    /// Batch edit multiple voxels efficiently
    pub fn set_voxels_batch(&mut self, edits: &[([f32; 3], Voxel)]) {
        // Group edits by chunk to minimize dirty marking overhead
        let mut edits_by_chunk: HashMap<ChunkCoord, Vec<([u32; 3], Voxel)>> = HashMap::new();

        for (world_pos, voxel) in edits {
            let voxel_idx = self.world_to_voxel(*world_pos);
            let chunk_coord = ChunkCoord::from_voxel(voxel_idx, Chunk::SIZE);
            let local = self.voxel_to_local(voxel_idx);

            edits_by_chunk
                .entry(chunk_coord)
                .or_default()
                .push((local, *voxel));
        }

        // Apply edits per chunk
        for (chunk_coord, chunk_edits) in edits_by_chunk {
            let chunk = self.chunks.entry(chunk_coord).or_insert_with(|| {
                Chunk::new(chunk_coord)
            });

            let mut combined_boundary = BoundaryFlags::default();

            for (local, voxel) in chunk_edits {
                let boundary = chunk.is_on_boundary(local[0], local[1], local[2]);
                combined_boundary.neg_x |= boundary.neg_x;
                combined_boundary.pos_x |= boundary.pos_x;
                combined_boundary.neg_y |= boundary.neg_y;
                combined_boundary.pos_y |= boundary.pos_y;
                combined_boundary.neg_z |= boundary.neg_z;
                combined_boundary.pos_z |= boundary.pos_z;

                chunk.set_voxel(local[0], local[1], local[2], voxel);
            }

            self.dirty_tracker.mark_dirty_with_neighbors(chunk_coord, combined_boundary);
            chunk.state = ChunkState::Dirty;
        }
    }
}
```

---

## 3. Rebuild Queue and Scheduling

### 3.1 Priority Queue

```rust
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Rebuild request with priority
#[derive(Clone, Debug)]
pub struct RebuildRequest {
    pub coord: ChunkCoord,
    pub priority: f32,  // Higher = more urgent (e.g., closer to camera)
    pub data_version: u64,
}

impl PartialEq for RebuildRequest {
    fn eq(&self, other: &Self) -> bool {
        self.coord == other.coord
    }
}

impl Eq for RebuildRequest {}

impl PartialOrd for RebuildRequest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.priority.partial_cmp(&other.priority)
    }
}

impl Ord for RebuildRequest {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

/// Priority queue for chunk rebuilds
pub struct RebuildQueue {
    queue: BinaryHeap<RebuildRequest>,
    /// Track which chunks are already in queue (for deduplication)
    in_queue: HashSet<ChunkCoord>,
}

impl RebuildQueue {
    pub fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
            in_queue: HashSet::new(),
        }
    }

    /// Add chunk to rebuild queue with priority
    pub fn enqueue(&mut self, coord: ChunkCoord, priority: f32, data_version: u64) {
        if self.in_queue.insert(coord) {
            self.queue.push(RebuildRequest {
                coord,
                priority,
                data_version,
            });
        }
        // If already in queue, we could update priority, but for simplicity
        // we just skip (the existing entry will be processed)
    }

    /// Pop highest-priority chunk
    pub fn pop(&mut self) -> Option<RebuildRequest> {
        if let Some(request) = self.queue.pop() {
            self.in_queue.remove(&request.coord);
            Some(request)
        } else {
            None
        }
    }

    /// Number of pending rebuilds
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Clear all pending rebuilds
    pub fn clear(&mut self) {
        self.queue.clear();
        self.in_queue.clear();
    }
}
```

### 3.2 Priority Calculation

```rust
impl ChunkManager {
    /// Calculate rebuild priority based on camera distance
    pub fn calculate_priority(&self, coord: ChunkCoord, camera_pos: [f32; 3]) -> f32 {
        let chunk_center = self.chunk_center_world(coord);
        let dx = chunk_center[0] - camera_pos[0];
        let dy = chunk_center[1] - camera_pos[1];
        let dz = chunk_center[2] - camera_pos[2];
        let distance_sq = dx * dx + dy * dy + dz * dz;

        // Invert so closer = higher priority
        // Add small epsilon to avoid division by zero
        1.0 / (distance_sq + 0.001)
    }

    /// Get world-space center of a chunk
    pub fn chunk_center_world(&self, coord: ChunkCoord) -> [f32; 3] {
        let half_size = Chunk::SIZE as f32 * self.voxel_size * 0.5;
        [
            coord.x as f32 * Chunk::SIZE as f32 * self.voxel_size + half_size,
            coord.y as f32 * Chunk::SIZE as f32 * self.voxel_size + half_size,
            coord.z as f32 * Chunk::SIZE as f32 * self.voxel_size + half_size,
        ]
    }
}
```

### 3.3 Frame Budget Scheduler

```rust
/// Configuration for rebuild scheduling
pub struct RebuildConfig {
    /// Maximum chunks to rebuild per frame
    pub max_chunks_per_frame: usize,

    /// Maximum time (ms) to spend rebuilding per frame
    pub max_time_per_frame_ms: f64,

    /// Whether to use async workers (if available)
    pub use_async_workers: bool,
}

impl Default for RebuildConfig {
    fn default() -> Self {
        Self {
            max_chunks_per_frame: 4,
            max_time_per_frame_ms: 8.0, // ~half a frame at 60fps
            use_async_workers: true,
        }
    }
}

impl ChunkManager {
    /// Process pending rebuilds within frame budget
    pub fn process_rebuilds(&mut self, camera_pos: [f32; 3]) -> RebuildStats {
        let start_time = instant::Instant::now();
        let mut stats = RebuildStats::default();

        // First, move dirty chunks to rebuild queue with priorities
        let dirty = self.dirty_tracker.take_dirty();
        for coord in dirty {
            if let Some(chunk) = self.chunks.get(&coord) {
                let priority = self.calculate_priority(coord, camera_pos);
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
                continue;
            };

            // Skip if data version changed (chunk was edited again)
            if chunk.data_version != request.data_version {
                // Re-enqueue with updated version
                let priority = self.calculate_priority(request.coord, camera_pos);
                self.rebuild_queue.enqueue(request.coord, priority, chunk.data_version);
                stats.version_mismatches += 1;
                continue;
            }

            // Perform rebuild
            let mesh = self.rebuild_chunk_mesh(chunk);
            stats.chunks_rebuilt += 1;
            stats.triangles_generated += mesh.triangle_count;

            // Update chunk state
            chunk.pending_mesh = Some(mesh);
            chunk.state = ChunkState::ReadyToSwap {
                data_version: chunk.data_version,
            };
        }

        stats.queue_remaining = self.rebuild_queue.len();
        stats.elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;
        stats
    }
}

#[derive(Default, Debug)]
pub struct RebuildStats {
    pub chunks_rebuilt: usize,
    pub triangles_generated: usize,
    pub version_mismatches: usize,
    pub queue_remaining: usize,
    pub time_budget_exceeded: bool,
    pub elapsed_ms: f64,
}
```

---

## 4. Mesh Swap Protocol

### 4.1 Swap Pending Meshes

```rust
impl ChunkManager {
    /// Swap all pending meshes into active slot
    /// Call this after process_rebuilds(), before rendering
    pub fn swap_pending_meshes(&mut self) -> SwapStats {
        let mut stats = SwapStats::default();

        for chunk in self.chunks.values_mut() {
            if let ChunkState::ReadyToSwap { data_version } = chunk.state {
                // Verify version still matches
                if data_version == chunk.data_version {
                    // Swap mesh
                    if let Some(pending) = chunk.pending_mesh.take() {
                        // Dispose old mesh (will be handled by render layer)
                        let old_mesh = chunk.mesh.replace(pending);
                        if old_mesh.is_some() {
                            stats.meshes_disposed += 1;
                        }
                        stats.meshes_swapped += 1;
                        chunk.state = ChunkState::Clean;
                    }
                } else {
                    // Version mismatch - data changed during meshing
                    // Discard pending mesh and mark dirty
                    chunk.pending_mesh = None;
                    chunk.state = ChunkState::Dirty;
                    self.dirty_tracker.mark_dirty(chunk.coord);
                    stats.version_conflicts += 1;
                }
            }
        }

        stats
    }
}

#[derive(Default, Debug)]
pub struct SwapStats {
    pub meshes_swapped: usize,
    pub meshes_disposed: usize,
    pub version_conflicts: usize,
}
```

---

## 5. Complete Frame Loop

```rust
impl ChunkManager {
    /// Call once per frame
    pub fn update(&mut self, camera_pos: [f32; 3]) -> FrameStats {
        // 1. Process pending async job completions (if using workers)
        let async_stats = self.poll_async_jobs();

        // 2. Process synchronous rebuilds within budget
        let rebuild_stats = self.process_rebuilds(camera_pos);

        // 3. Swap completed meshes
        let swap_stats = self.swap_pending_meshes();

        // 4. Return combined stats
        FrameStats {
            async_jobs_completed: async_stats.completed,
            chunks_rebuilt: rebuild_stats.chunks_rebuilt,
            meshes_swapped: swap_stats.meshes_swapped,
            queue_remaining: rebuild_stats.queue_remaining,
            rebuild_time_ms: rebuild_stats.elapsed_ms,
        }
    }
}

#[derive(Default, Debug)]
pub struct FrameStats {
    pub async_jobs_completed: usize,
    pub chunks_rebuilt: usize,
    pub meshes_swapped: usize,
    pub queue_remaining: usize,
    pub rebuild_time_ms: f64,
}
```

---

## 6. Debug Inspection

```rust
impl ChunkManager {
    /// Get debug info for all chunks
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

            if let Some(mesh) = &chunk.mesh {
                info.total_triangles += mesh.triangle_count;
                info.total_vertices += mesh.vertex_count;
            }
        }

        info.queue_size = self.rebuild_queue.len();
        info
    }
}

#[derive(Default, Debug)]
pub struct ChunkDebugInfo {
    pub total_chunks: usize,
    pub clean_chunks: usize,
    pub dirty_chunks: usize,
    pub meshing_chunks: usize,
    pub ready_to_swap_chunks: usize,
    pub queue_size: usize,
    pub total_triangles: usize,
    pub total_vertices: usize,
}
```

---

## 7. Error Handling

### 7.1 Error Types

```rust
#[derive(Debug)]
pub enum ChunkError {
    /// Chunk coordinates out of valid range
    OutOfBounds(ChunkCoord),

    /// Mesh generation failed
    MeshingFailed {
        coord: ChunkCoord,
        reason: String,
    },

    /// Buffer allocation failed
    AllocationFailed {
        coord: ChunkCoord,
        requested_bytes: usize,
    },
}
```

### 7.2 Recovery Strategy

```rust
impl ChunkManager {
    /// Handle meshing failure with graceful degradation
    fn handle_mesh_failure(&mut self, coord: ChunkCoord, error: ChunkError) {
        log::error!("Chunk mesh failed: {:?}", error);

        if let Some(chunk) = self.chunks.get_mut(&coord) {
            // Keep existing mesh if available
            if chunk.mesh.is_some() {
                chunk.state = ChunkState::Clean; // Use stale mesh
            } else {
                // No mesh at all - create empty placeholder
                chunk.mesh = Some(ChunkMesh::empty());
                chunk.state = ChunkState::Clean;
            }
        }
    }
}
```

---

## Summary

| Component | Responsibility |
|-----------|----------------|
| `ChunkCoord` | Chunk identification and neighbor calculation |
| `ChunkState` | Lifecycle state machine |
| `DirtyTracker` | Deduped dirty chunk tracking with boundary awareness |
| `RebuildQueue` | Priority-ordered rebuild scheduling |
| `RebuildConfig` | Frame budget configuration |
| `ChunkManager` | Orchestrates the complete update loop |

This system ensures:
- Edits are responsive (dirty marking is O(1))
- No redundant rebuilds (deduplication)
- Frame rate stability (budgeted processing)
- Data consistency (version checking)
- Debuggability (comprehensive stats)
