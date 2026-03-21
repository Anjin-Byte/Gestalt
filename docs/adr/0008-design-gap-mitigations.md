# 0008 - Design Gap Mitigations

## Status
Proposed

## Context

A review of the voxel mesh system documentation identified 12 design gaps that could cause stability, correctness, or performance issues. This ADR documents the mitigations for each gap.

---

## Gap 1: Unbounded Worst-Case Memory / Mesh Explosion

**Problem:** Pathological inputs (checkerboard patterns) can produce mesh sizes in the tens of MB per chunk. No guardrails exist.

**Impact:** JS heap exhaustion, GPU memory overflow, browser tab crash.

**Decision:** Implement mesh complexity cap with fallback rendering.

### Mitigation

```rust
/// Maximum triangles per chunk before fallback
pub const MAX_TRIANGLES_PER_CHUNK: usize = 100_000;

/// Maximum vertices per chunk (4 per quad × MAX_TRIANGLES/2)
pub const MAX_VERTICES_PER_CHUNK: usize = 200_000;

pub enum MeshResult {
    /// Normal greedy mesh output
    Mesh(MeshOutput),
    /// Complexity exceeded - use point cloud fallback
    FallbackToPoints {
        positions: Vec<f32>,
        reason: ComplexityReason,
    },
}

pub enum ComplexityReason {
    TriangleCountExceeded { count: usize, max: usize },
    MemoryBudgetExceeded { bytes: usize, max: usize },
}

pub fn mesh_chunk_with_fallback(
    chunk: &BinaryChunk,
    voxel_size: f32,
    origin: [f32; 3],
) -> MeshResult {
    // Early detection: estimate complexity from face mask population
    let estimated_faces = estimate_visible_faces(chunk);
    if estimated_faces > MAX_TRIANGLES_PER_CHUNK {
        return MeshResult::FallbackToPoints {
            positions: extract_solid_positions(chunk, voxel_size, origin),
            reason: ComplexityReason::TriangleCountExceeded {
                count: estimated_faces,
                max: MAX_TRIANGLES_PER_CHUNK,
            },
        };
    }

    let output = mesh_chunk(chunk, voxel_size, origin);

    if output.triangle_count() > MAX_TRIANGLES_PER_CHUNK {
        MeshResult::FallbackToPoints {
            positions: extract_solid_positions(chunk, voxel_size, origin),
            reason: ComplexityReason::TriangleCountExceeded {
                count: output.triangle_count(),
                max: MAX_TRIANGLES_PER_CHUNK,
            },
        }
    } else {
        MeshResult::Mesh(output)
    }
}

fn estimate_visible_faces(chunk: &BinaryChunk) -> usize {
    // Count set bits in opaque mask as upper bound
    // Each voxel can contribute at most 6 faces
    let solid_count: usize = chunk.opaque_mask.iter()
        .map(|col| col.count_ones() as usize)
        .sum();
    solid_count * 6
}
```

### TypeScript Integration

```typescript
type ChunkRenderMode =
    | { kind: 'mesh'; geometry: BufferGeometry }
    | { kind: 'points'; geometry: BufferGeometry; reason: string };

function handleMeshResult(result: MeshResult, chunkId: ChunkId): ChunkRenderMode {
    if ('positions' in result && 'reason' in result) {
        logger.warn('Chunk exceeded complexity limit, using points', {
            chunkId,
            reason: result.reason,
        });
        return {
            kind: 'points',
            geometry: buildPointGeometry(result.positions),
            reason: result.reason,
        };
    }
    return { kind: 'mesh', geometry: buildMeshGeometry(result) };
}
```

---

## Gap 2: No Backpressure Strategy for Rapid Edits

**Problem:** No policy for when edits arrive faster than rebuilds. Unbounded queue growth and frame spikes.

**Impact:** Memory growth, UI freeze, dropped frames.

**Decision:** Implement edit coalescing window + queue depth limit + oldest-drop policy.

### Mitigation

```typescript
interface BackpressureConfig {
    /** Coalesce edits within this window (ms) */
    coalesceWindowMs: number;
    /** Maximum pending rebuilds before dropping */
    maxQueueDepth: number;
    /** Drop strategy when queue full */
    dropPolicy: 'oldest' | 'farthest' | 'lowest_priority';
}

const DEFAULT_BACKPRESSURE: BackpressureConfig = {
    coalesceWindowMs: 16,  // ~1 frame at 60fps
    maxQueueDepth: 64,     // 64 chunks max pending
    dropPolicy: 'farthest',
};

class RebuildScheduler {
    private pendingEdits = new Map<ChunkId, {
        firstEdit: number;
        editCount: number;
        positions: Set<string>;
    }>();

    private rebuildQueue: ChunkId[] = [];
    private config: BackpressureConfig;

    markDirty(chunkId: ChunkId, positions: VoxelPos[]): void {
        const now = performance.now();
        const existing = this.pendingEdits.get(chunkId);

        if (existing) {
            // Coalesce into existing pending edit
            existing.editCount++;
            for (const pos of positions) {
                existing.positions.add(posKey(pos));
            }
        } else {
            // New pending edit
            this.pendingEdits.set(chunkId, {
                firstEdit: now,
                editCount: 1,
                positions: new Set(positions.map(posKey)),
            });
        }

        this.scheduleFlush();
    }

    private scheduleFlush(): void {
        // Flush coalesced edits after window
        setTimeout(() => this.flushPendingEdits(), this.config.coalesceWindowMs);
    }

    private flushPendingEdits(): void {
        const now = performance.now();

        for (const [chunkId, pending] of this.pendingEdits) {
            if (now - pending.firstEdit >= this.config.coalesceWindowMs) {
                this.enqueueRebuild(chunkId);
                this.pendingEdits.delete(chunkId);
            }
        }
    }

    private enqueueRebuild(chunkId: ChunkId): void {
        // Deduplicate
        if (this.rebuildQueue.includes(chunkId)) return;

        // Apply backpressure if queue full
        while (this.rebuildQueue.length >= this.config.maxQueueDepth) {
            const dropped = this.dropOne();
            logger.warn('Rebuild queue full, dropped chunk', { dropped });
        }

        this.rebuildQueue.push(chunkId);
    }

    private dropOne(): ChunkId {
        switch (this.config.dropPolicy) {
            case 'oldest':
                return this.rebuildQueue.shift()!;
            case 'farthest':
                return this.dropFarthestFromCamera();
            case 'lowest_priority':
                return this.dropLowestPriority();
        }
    }

    private dropFarthestFromCamera(): ChunkId {
        let maxDist = -1;
        let maxIdx = 0;
        const camPos = this.getCameraChunkCoord();

        for (let i = 0; i < this.rebuildQueue.length; i++) {
            const coord = chunkCoord(this.rebuildQueue[i]);
            const dist = Math.abs(coord.x - camPos.x)
                       + Math.abs(coord.y - camPos.y)
                       + Math.abs(coord.z - camPos.z);
            if (dist > maxDist) {
                maxDist = dist;
                maxIdx = i;
            }
        }

        return this.rebuildQueue.splice(maxIdx, 1)[0];
    }
}
```

---

## Gap 3: Async Meshing vs Mutable Chunk Data Race

**Problem:** No specification for how chunk data is snapshotted for async workers. Mid-mesh mutation could corrupt results.

**Impact:** Visual artifacts, incorrect geometry, undefined behavior.

**Decision:** Copy-on-write snapshot for worker hand-off.

### Mitigation

```rust
/// Immutable snapshot of chunk data for async meshing
#[derive(Clone)]
pub struct ChunkSnapshot {
    pub opaque_mask: Box<[u64; CS_P2]>,
    pub materials: Box<[MaterialId; CS_P3]>,
    pub version: u64,
    pub coord: [i32; 3],
}

impl ChunkSnapshot {
    /// Create snapshot from mutable chunk (copies data)
    pub fn from_chunk(chunk: &BinaryChunk, version: u64, coord: [i32; 3]) -> Self {
        Self {
            opaque_mask: Box::new(chunk.opaque_mask),
            materials: Box::new(chunk.materials),
            version,
            coord,
        }
    }

    /// Size in bytes (for memory budgeting)
    pub const SIZE_BYTES: usize = CS_P2 * 8 + CS_P3 * 2 + 8 + 12;
}
```

### Worker Protocol

```typescript
// Main thread
function dispatchMeshJob(chunk: Chunk): void {
    // Take snapshot before dispatching
    const snapshot: ChunkSnapshot = {
        opaqueMask: chunk.opaqueMask.slice(),  // Copy
        materials: chunk.materials.slice(),     // Copy
        version: chunk.version,
        coord: chunk.coord,
    };

    // Transfer ownership to worker (zero-copy for TypedArrays)
    worker.postMessage({
        type: 'mesh',
        snapshot,
    }, [snapshot.opaqueMask.buffer, snapshot.materials.buffer]);

    chunk.state = { kind: 'meshing', pendingVersion: snapshot.version };
}

// Worker thread
self.onmessage = (e) => {
    if (e.data.type === 'mesh') {
        const { snapshot } = e.data;
        const result = meshChunk(snapshot);

        self.postMessage({
            type: 'mesh_complete',
            coord: snapshot.coord,
            version: snapshot.version,
            result,
        }, [result.positions.buffer, result.normals.buffer, result.indices.buffer]);
    }
};

// Main thread - receive result
function handleMeshComplete(msg: MeshCompleteMessage): void {
    const chunk = chunks.get(chunkId(msg.coord));
    if (!chunk) return;

    // Version check - discard if chunk was modified during meshing
    if (chunk.version !== msg.version) {
        logger.debug('Discarding stale mesh result', {
            coord: msg.coord,
            meshVersion: msg.version,
            currentVersion: chunk.version,
        });
        return;
    }

    chunk.state = {
        kind: 'ready_to_swap',
        oldMesh: chunk.mesh,
        newMesh: buildGeometry(msg.result),
        version: msg.version,
    };
}
```

---

## Gap 4: Dense Voxel Storage Only

**Problem:** Dense `[u8; 64³]` storage wastes memory for sparse worlds (empty chunks still allocate 256KB+).

**Impact:** High memory usage for large sparse worlds.

**Decision:** Hybrid storage with sparse representation for low fill ratios.

### Mitigation

```rust
/// Chunk storage that adapts based on fill ratio
pub enum ChunkStorage {
    /// Dense storage for fill ratio >= 10%
    Dense(BinaryChunk),
    /// Sparse storage for fill ratio < 10%
    Sparse(SparseChunk),
    /// Empty chunk (no allocation)
    Empty,
}

pub struct SparseChunk {
    /// Map from voxel position to material
    voxels: HashMap<u32, MaterialId>,  // Position packed as x + y*64 + z*4096
}

impl SparseChunk {
    const SPARSE_THRESHOLD: f32 = 0.10;  // 10% fill ratio

    pub fn from_dense(chunk: &BinaryChunk) -> ChunkStorage {
        let solid_count = chunk.opaque_mask.iter()
            .map(|col| col.count_ones() as usize)
            .sum::<usize>();

        let fill_ratio = solid_count as f32 / (CS * CS * CS) as f32;

        if solid_count == 0 {
            ChunkStorage::Empty
        } else if fill_ratio < Self::SPARSE_THRESHOLD {
            let mut sparse = SparseChunk { voxels: HashMap::new() };
            for x in 0..CS {
                for z in 0..CS {
                    let col = chunk.opaque_mask[(x + 1) * CS_P + (z + 1)];
                    let mut bits = col >> 1;
                    let mut y = 0;
                    while bits != 0 {
                        if bits & 1 != 0 {
                            let pos = (x + y * CS + z * CS * CS) as u32;
                            let mat = chunk.get_material(x + 1, y + 1, z + 1);
                            sparse.voxels.insert(pos, mat);
                        }
                        bits >>= 1;
                        y += 1;
                    }
                }
            }
            ChunkStorage::Sparse(sparse)
        } else {
            ChunkStorage::Dense(chunk.clone())
        }
    }

    /// Convert to dense for meshing (sparse chunks mesh less efficiently)
    pub fn to_dense(&self) -> BinaryChunk {
        let mut chunk = BinaryChunk::new();
        for (&pos, &mat) in &self.voxels {
            let x = (pos % CS as u32) as usize;
            let y = ((pos / CS as u32) % CS as u32) as usize;
            let z = (pos / (CS * CS) as u32) as usize;
            chunk.set(x + 1, y + 1, z + 1, mat);
        }
        chunk
    }

    pub fn memory_bytes(&self) -> usize {
        // HashMap overhead + entry size × count
        std::mem::size_of::<Self>() + self.voxels.len() * 6
    }
}

impl ChunkStorage {
    pub fn memory_bytes(&self) -> usize {
        match self {
            ChunkStorage::Dense(_) => std::mem::size_of::<BinaryChunk>(),
            ChunkStorage::Sparse(s) => s.memory_bytes(),
            ChunkStorage::Empty => 0,
        }
    }
}
```

---

## Gap 5: Chunk Size Lock-In (64³)

**Problem:** System tightly coupled to 64³. No flexibility for different scales or LOD.

**Impact:** Cannot optimize for different use cases (large terrain vs fine detail).

**Decision:** Accept 64³ as hard constraint; document rationale and escape hatches.

### Rationale

The 64³ chunk size is a fundamental constraint driven by:

1. **u64 bitmask columns** - The binary greedy meshing algorithm processes 64 voxels per bitwise operation
2. **WASM performance** - 64-bit operations are native; other sizes require emulation
3. **Cache efficiency** - 64×64 = 4096 columns fits well in L1 cache
4. **Memory alignment** - 64³ × 2 bytes = 512KB, a power-of-two allocation

### Documented Constraints

```rust
/// HARD CONSTRAINT: Chunk size cannot be changed without algorithm rewrite.
/// See ADR-0004 and ADR-0008 for rationale.
pub const CHUNK_SIZE: usize = 64;

// Compile-time assertion
const _: () = assert!(CHUNK_SIZE == 64, "Chunk size must be 64 for binary meshing");
```

### Escape Hatches

For different scales, use these alternatives instead of changing chunk size:

| Need | Solution |
|------|----------|
| Larger terrain | More chunks, not bigger chunks |
| Finer detail | Smaller `voxel_size`, same chunk dimensions |
| LOD | Point decimation within 64³ chunks (ADR-0006) |
| Streaming | 64³ remains the atomic load/unload unit |

---

## Gap 6: Float→Voxel Snapping Errors

**Problem:** Naïve `floor(world_pos / voxel_size)` has floating-point precision issues at boundaries.

**Impact:** Off-by-one voxel placement, inconsistent results across platforms.

**Decision:** Canonical coordinate conversion with epsilon tolerance and explicit rounding.

### Mitigation

```rust
/// Epsilon for floating-point comparisons (half a voxel's precision)
const COORD_EPSILON: f32 = 1e-5;

/// Convert world position to voxel index with robust rounding
pub fn world_to_voxel(
    world_pos: [f32; 3],
    voxel_size: f32,
    world_origin: [f32; 3],
) -> [i32; 3] {
    let inv_size = 1.0 / voxel_size;

    [
        robust_floor((world_pos[0] - world_origin[0]) * inv_size),
        robust_floor((world_pos[1] - world_origin[1]) * inv_size),
        robust_floor((world_pos[2] - world_origin[2]) * inv_size),
    ]
}

/// Floor with epsilon tolerance for values very close to integers
fn robust_floor(x: f32) -> i32 {
    let rounded = x.round();
    if (x - rounded).abs() < COORD_EPSILON {
        rounded as i32
    } else {
        x.floor() as i32
    }
}

/// Convert voxel index to world position (center of voxel)
pub fn voxel_to_world(
    voxel_idx: [i32; 3],
    voxel_size: f32,
    world_origin: [f32; 3],
) -> [f32; 3] {
    [
        world_origin[0] + (voxel_idx[0] as f32 + 0.5) * voxel_size,
        world_origin[1] + (voxel_idx[1] as f32 + 0.5) * voxel_size,
        world_origin[2] + (voxel_idx[2] as f32 + 0.5) * voxel_size,
    ]
}

/// Convert voxel index to chunk coordinate
pub fn voxel_to_chunk(voxel_idx: [i32; 3]) -> [i32; 3] {
    [
        voxel_idx[0].div_euclid(CS as i32),
        voxel_idx[1].div_euclid(CS as i32),
        voxel_idx[2].div_euclid(CS as i32),
    ]
}

/// Convert voxel index to local position within chunk
pub fn voxel_to_local(voxel_idx: [i32; 3]) -> [usize; 3] {
    [
        voxel_idx[0].rem_euclid(CS as i32) as usize + 1,  // +1 for padding
        voxel_idx[1].rem_euclid(CS as i32) as usize + 1,
        voxel_idx[2].rem_euclid(CS as i32) as usize + 1,
    ]
}
```

### TypeScript Equivalent

```typescript
const COORD_EPSILON = 1e-5;

function robustFloor(x: number): number {
    const rounded = Math.round(x);
    if (Math.abs(x - rounded) < COORD_EPSILON) {
        return rounded;
    }
    return Math.floor(x);
}

function worldToVoxel(
    worldPos: Vec3,
    voxelSize: number,
    worldOrigin: Vec3
): [number, number, number] {
    const invSize = 1 / voxelSize;
    return [
        robustFloor((worldPos[0] - worldOrigin[0]) * invSize),
        robustFloor((worldPos[1] - worldOrigin[1]) * invSize),
        robustFloor((worldPos[2] - worldOrigin[2]) * invSize),
    ];
}
```

---

## Gap 7: Missing Neighbor Policy

**Problem:** Boundary padding treats missing neighbors as empty, creating false boundary faces during streaming.

**Impact:** Visible holes at chunk boundaries before neighbors load.

**Decision:** Add `Unknown` neighbor state with conservative (defer meshing) policy.

### Mitigation

```typescript
type NeighborState =
    | { kind: 'loaded'; chunk: Chunk }
    | { kind: 'empty' }      // Known to be empty (no voxels)
    | { kind: 'unknown' };   // Not yet loaded

interface BoundaryPolicy {
    /** How to handle unknown neighbors during meshing */
    unknownNeighbor: 'defer' | 'assume_solid' | 'assume_empty';
}

const DEFAULT_BOUNDARY_POLICY: BoundaryPolicy = {
    unknownNeighbor: 'defer',  // Wait for neighbor to load
};

class ChunkManager {
    private neighbors = new Map<ChunkId, Map<FaceDir, NeighborState>>();
    private boundaryPolicy: BoundaryPolicy;

    canMesh(chunkId: ChunkId): boolean {
        if (this.boundaryPolicy.unknownNeighbor === 'defer') {
            // Check all 6 neighbors are known
            const neighborStates = this.neighbors.get(chunkId);
            if (!neighborStates) return false;

            for (const state of neighborStates.values()) {
                if (state.kind === 'unknown') {
                    return false;  // Defer until neighbor loads
                }
            }
        }
        return true;
    }

    getPaddedChunkData(chunkId: ChunkId): PaddedChunkView {
        const chunk = this.chunks.get(chunkId)!;
        const view = new PaddedChunkView(chunk);

        const neighborStates = this.neighbors.get(chunkId)!;
        for (const [face, state] of neighborStates) {
            switch (state.kind) {
                case 'loaded':
                    view.setBoundaryFromNeighbor(face, state.chunk);
                    break;
                case 'empty':
                    view.setBoundaryEmpty(face);
                    break;
                case 'unknown':
                    // Policy-dependent
                    if (this.boundaryPolicy.unknownNeighbor === 'assume_solid') {
                        view.setBoundarySolid(face);  // Conservative: no boundary faces
                    } else {
                        view.setBoundaryEmpty(face);  // Optimistic: show boundary faces
                    }
                    break;
            }
        }

        return view;
    }

    onNeighborLoaded(chunkId: ChunkId, neighborId: ChunkId, face: FaceDir): void {
        const states = this.neighbors.get(chunkId);
        if (states) {
            const neighborChunk = this.chunks.get(neighborId);
            states.set(face, neighborChunk
                ? { kind: 'loaded', chunk: neighborChunk }
                : { kind: 'empty' }
            );

            // If we were deferring, check if we can now mesh
            const chunk = this.chunks.get(chunkId);
            if (chunk?.state.kind === 'waiting_for_neighbors' && this.canMesh(chunkId)) {
                this.enqueueMesh(chunkId);
            }
        }
    }
}
```

---

## Gap 8: Preallocation Not Actually Used

**Problem:** `threejs-buffer-management.md` advocates preallocation and `drawRange`, but the code creates new `BufferGeometry` each update.

**Impact:** GC pressure, allocation overhead, defeats stated performance goals.

**Decision:** Implement true preallocation with fixed-size buffers and drawRange.

### Mitigation

```typescript
interface PreallocatedMesh {
    geometry: BufferGeometry;
    mesh: Mesh;
    /** Maximum vertices this buffer can hold */
    maxVertices: number;
    /** Maximum indices this buffer can hold */
    maxIndices: number;
    /** Currently used vertices */
    usedVertices: number;
    /** Currently used indices */
    usedIndices: number;
}

class ChunkMeshPool {
    // Size tiers for different chunk complexities
    private static readonly SIZE_TIERS = [
        { maxVertices: 1_000, maxIndices: 2_000 },      // Simple
        { maxVertices: 10_000, maxIndices: 20_000 },   // Typical
        { maxVertices: 50_000, maxIndices: 100_000 },  // Complex
    ];

    private pools: Map<number, PreallocatedMesh[]> = new Map();
    private active: Map<ChunkId, PreallocatedMesh> = new Map();

    constructor() {
        // Preallocate buffers for each tier
        for (const tier of ChunkMeshPool.SIZE_TIERS) {
            this.pools.set(tier.maxVertices, []);
            for (let i = 0; i < 8; i++) {  // 8 per tier initially
                this.pools.get(tier.maxVertices)!.push(this.createBuffer(tier));
            }
        }
    }

    private createBuffer(tier: { maxVertices: number; maxIndices: number }): PreallocatedMesh {
        const geometry = new BufferGeometry();

        // Preallocate with maximum size
        const positions = new Float32Array(tier.maxVertices * 3);
        const normals = new Float32Array(tier.maxVertices * 3);
        const indices = new Uint32Array(tier.maxIndices);

        geometry.setAttribute('position', new BufferAttribute(positions, 3));
        geometry.setAttribute('normal', new BufferAttribute(normals, 3));
        geometry.setIndex(new BufferAttribute(indices, 1));

        // Start with zero draw range
        geometry.setDrawRange(0, 0);

        const mesh = new Mesh(geometry, this.sharedMaterial);
        mesh.frustumCulled = true;
        mesh.visible = false;

        return {
            geometry,
            mesh,
            maxVertices: tier.maxVertices,
            maxIndices: tier.maxIndices,
            usedVertices: 0,
            usedIndices: 0,
        };
    }

    acquire(chunkId: ChunkId, vertexCount: number, indexCount: number): PreallocatedMesh | null {
        // Find smallest tier that fits
        const tier = ChunkMeshPool.SIZE_TIERS.find(
            t => t.maxVertices >= vertexCount && t.maxIndices >= indexCount
        );

        if (!tier) {
            logger.warn('Mesh too large for any pool tier', { vertexCount, indexCount });
            return null;
        }

        const pool = this.pools.get(tier.maxVertices)!;
        let buffer = pool.pop();

        if (!buffer) {
            // Pool exhausted, create new buffer
            buffer = this.createBuffer(tier);
        }

        this.active.set(chunkId, buffer);
        return buffer;
    }

    update(chunkId: ChunkId, meshData: MeshOutput): boolean {
        const buffer = this.active.get(chunkId);
        if (!buffer) return false;

        const vertexCount = meshData.positions.length / 3;
        const indexCount = meshData.indices.length;

        if (vertexCount > buffer.maxVertices || indexCount > buffer.maxIndices) {
            // Need larger buffer - release and reacquire
            this.release(chunkId);
            const newBuffer = this.acquire(chunkId, vertexCount, indexCount);
            if (!newBuffer) return false;
            return this.update(chunkId, meshData);
        }

        // Update buffer contents (no reallocation!)
        const posAttr = buffer.geometry.getAttribute('position') as BufferAttribute;
        const normAttr = buffer.geometry.getAttribute('normal') as BufferAttribute;
        const idxAttr = buffer.geometry.getIndex()!;

        (posAttr.array as Float32Array).set(meshData.positions);
        (normAttr.array as Float32Array).set(meshData.normals);
        (idxAttr.array as Uint32Array).set(meshData.indices);

        posAttr.needsUpdate = true;
        normAttr.needsUpdate = true;
        idxAttr.needsUpdate = true;

        // Update draw range to actual data size
        buffer.geometry.setDrawRange(0, indexCount);
        buffer.usedVertices = vertexCount;
        buffer.usedIndices = indexCount;

        // Update bounding volumes
        buffer.geometry.computeBoundingBox();
        buffer.geometry.computeBoundingSphere();

        buffer.mesh.visible = true;
        return true;
    }

    release(chunkId: ChunkId): void {
        const buffer = this.active.get(chunkId);
        if (!buffer) return;

        buffer.mesh.visible = false;
        buffer.geometry.setDrawRange(0, 0);
        buffer.usedVertices = 0;
        buffer.usedIndices = 0;

        // Return to pool
        this.pools.get(buffer.maxVertices)!.push(buffer);
        this.active.delete(chunkId);
    }
}
```

---

## Gap 9: Material Strategy Unintegrated

**Problem:** ADR-0007 proposes materials but isn't integrated into the rendering pipeline.

**Impact:** Incomplete feature, unclear implementation path.

**Decision:** Integrate ADR-0007 into chunk mesh pool and rendering.

### Integration Points

1. **ChunkMeshPool** - Add UV and materialId attributes to preallocated buffers
2. **MeshOutput** - Already includes `uvs` and `material_ids` (ADR-0007)
3. **Material** - Use shader material from ADR-0007

```typescript
// In ChunkMeshPool.createBuffer()
if (this.materialMode === 'textured') {
    const uvs = new Float32Array(tier.maxVertices * 2);
    const materialIds = new Uint16Array(tier.maxVertices);
    geometry.setAttribute('uv', new BufferAttribute(uvs, 2));
    geometry.setAttribute('materialId', new BufferAttribute(materialIds, 1));
}

// In ChunkMeshPool.update()
if (meshData.uvs && meshData.material_ids) {
    const uvAttr = buffer.geometry.getAttribute('uv') as BufferAttribute;
    const matAttr = buffer.geometry.getAttribute('materialId') as BufferAttribute;
    (uvAttr.array as Float32Array).set(meshData.uvs);
    (matAttr.array as Uint16Array).set(meshData.material_ids);
    uvAttr.needsUpdate = true;
    matAttr.needsUpdate = true;
}
```

---

## Gap 10: LOD Integration Undefined

**Problem:** ADR-0006 defines LOD strategy but no integration with chunk system.

**Impact:** LOD remains theoretical, no clear implementation path.

**Decision:** Add LOD distance bands to rebuild queue priority and render decisions.

### Integration Points

```typescript
interface LODConfig {
    bands: LODBand[];
}

interface LODBand {
    minDistance: number;  // Chunks from camera
    maxDistance: number;
    renderMode: 'mesh' | 'points' | 'none';
    meshPriority: number;  // Lower = higher priority in queue
    pointDecimation?: number;  // Skip every N voxels for points
}

const DEFAULT_LOD: LODConfig = {
    bands: [
        { minDistance: 0, maxDistance: 4, renderMode: 'mesh', meshPriority: 0 },
        { minDistance: 4, maxDistance: 8, renderMode: 'points', meshPriority: 10, pointDecimation: 2 },
        { minDistance: 8, maxDistance: Infinity, renderMode: 'none', meshPriority: 100 },
    ],
};

class RebuildScheduler {
    private lodConfig: LODConfig;

    getPriority(chunkId: ChunkId): number {
        const coord = chunkCoord(chunkId);
        const camCoord = this.getCameraChunkCoord();
        const distance = Math.max(
            Math.abs(coord.x - camCoord.x),
            Math.abs(coord.y - camCoord.y),
            Math.abs(coord.z - camCoord.z)
        );

        const band = this.lodConfig.bands.find(
            b => distance >= b.minDistance && distance < b.maxDistance
        );

        return band?.meshPriority ?? 1000;
    }

    shouldMesh(chunkId: ChunkId): boolean {
        const band = this.getLODBand(chunkId);
        return band?.renderMode === 'mesh';
    }

    getRenderMode(chunkId: ChunkId): 'mesh' | 'points' | 'none' {
        const band = this.getLODBand(chunkId);
        return band?.renderMode ?? 'none';
    }
}
```

---

## Gap 11: Threading/Worker Strategy Deferred Too Late

**Problem:** Meshing workers deferred to Phase 5, but early phases assume sync meshing blocks main thread.

**Impact:** Jank during Phase 1-4 testing, architectural retrofit required.

**Decision:** Design worker-ready API in Phase 1; implement workers in Phase 5.

### Worker-Ready API Design

```typescript
// Abstract interface allows sync or async implementation
interface MeshingBackend {
    mesh(snapshot: ChunkSnapshot): Promise<MeshOutput>;
    dispose(): void;
}

// Phase 1-4: Synchronous implementation (blocks main thread)
class SyncMeshingBackend implements MeshingBackend {
    private wasm: WasmGreedyMesher;

    async mesh(snapshot: ChunkSnapshot): Promise<MeshOutput> {
        // Runs on main thread, but API is async for future compatibility
        return this.wasm.meshChunk(snapshot);
    }

    dispose(): void {}
}

// Phase 5: Worker pool implementation
class WorkerMeshingBackend implements MeshingBackend {
    private workers: Worker[] = [];
    private pending = new Map<number, { resolve: Function; reject: Function }>();
    private nextJobId = 0;

    constructor(workerCount = navigator.hardwareConcurrency || 4) {
        for (let i = 0; i < workerCount; i++) {
            const worker = new Worker(new URL('./meshWorker.ts', import.meta.url));
            worker.onmessage = (e) => this.handleResult(e.data);
            this.workers.push(worker);
        }
    }

    async mesh(snapshot: ChunkSnapshot): Promise<MeshOutput> {
        const jobId = this.nextJobId++;
        const worker = this.workers[jobId % this.workers.length];

        return new Promise((resolve, reject) => {
            this.pending.set(jobId, { resolve, reject });
            worker.postMessage({ jobId, snapshot }, [
                snapshot.opaqueMask.buffer,
                snapshot.materials.buffer,
            ]);
        });
    }

    private handleResult(data: { jobId: number; result: MeshOutput }): void {
        const pending = this.pending.get(data.jobId);
        if (pending) {
            pending.resolve(data.result);
            this.pending.delete(data.jobId);
        }
    }

    dispose(): void {
        for (const worker of this.workers) {
            worker.terminate();
        }
    }
}

// Usage (same API regardless of backend)
const backend: MeshingBackend = USE_WORKERS
    ? new WorkerMeshingBackend()
    : new SyncMeshingBackend();

const mesh = await backend.mesh(snapshot);
```

---

## Gap 12: Determinism Requirement vs Floating Inputs

**Problem:** Architecture requires byte-identical output, but float-based conversion is platform-dependent.

**Impact:** Non-reproducible results, test flakiness.

**Decision:** Use fixed-point intermediate for coordinate conversion; document determinism boundaries.

### Mitigation

```rust
/// Fixed-point voxel coordinate (1/256 voxel precision)
/// Eliminates floating-point platform differences
pub struct FixedVoxelCoord {
    /// 256 units = 1 voxel
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl FixedVoxelCoord {
    const SCALE: f32 = 256.0;

    pub fn from_world(world: [f32; 3], voxel_size: f32, origin: [f32; 3]) -> Self {
        let inv_size = Self::SCALE / voxel_size;
        Self {
            x: ((world[0] - origin[0]) * inv_size).round() as i32,
            y: ((world[1] - origin[1]) * inv_size).round() as i32,
            z: ((world[2] - origin[2]) * inv_size).round() as i32,
        }
    }

    pub fn to_voxel(&self) -> [i32; 3] {
        [
            self.x.div_euclid(256),
            self.y.div_euclid(256),
            self.z.div_euclid(256),
        ]
    }
}
```

### Determinism Boundaries

| Layer | Deterministic? | Notes |
|-------|----------------|-------|
| Voxel indices (after conversion) | Yes | Fixed-point intermediate |
| Meshing algorithm | Yes | Integer bitwise operations |
| Vertex positions | Yes | Integer voxel coords × voxel_size |
| Triangle order | Yes | Deterministic iteration order |
| Floating-point input | **No** | Platform-dependent; use fixed-point for cross-platform |

---

## Consequences

### Positive

- **Stability**: Memory caps prevent crashes from pathological inputs
- **Responsiveness**: Backpressure keeps UI responsive under heavy edits
- **Correctness**: Snapshot isolation eliminates data races
- **Efficiency**: True preallocation reduces GC pressure
- **Flexibility**: Worker-ready API enables future parallelism

### Negative

- **Complexity**: More code paths (fallbacks, policies, tiers)
- **Memory overhead**: Snapshot copies, preallocated buffers
- **Deferred neighbors**: Slower initial world load with conservative policy

### Constraints Introduced

- Maximum ~100K triangles per chunk
- Neighbor loading required for boundary-correct meshing
- Preallocated buffer tiers limit mesh sizes

## References

- [ADR-0003](0003-binary-greedy-meshing.md) - Binary greedy meshing
- [ADR-0004](0004-chunk-size-64.md) - 64³ chunk size rationale
- [ADR-0006](0006-lod-strategy.md) - LOD strategy
- [ADR-0007](0007-material-strategy.md) - Material strategy
- [chunk-management-system.md](../chunk-management-system.md) - State machine
- [threejs-buffer-management.md](../threejs-buffer-management.md) - Buffer lifecycle
