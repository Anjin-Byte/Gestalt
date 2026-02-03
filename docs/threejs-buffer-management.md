# Three.js Buffer Management

Technical specification for managing Three.js BufferGeometry, mesh objects, and GPU buffer lifecycle in the voxel rendering system.

## 1. Design Principles

### 1.1 Stable Object References
- **Mesh objects are persistent**: Create once, reuse across rebuilds
- **Geometry is swapped, not Mesh**: Replace `mesh.geometry`, don't recreate Mesh
- **Scene graph stability**: Avoid add/remove churn on the scene

### 1.2 Buffer Lifecycle
- **Preallocate buffers**: Size buffers for expected capacity
- **Use drawRange**: Control visible portion without reallocating
- **Explicit disposal**: Always dispose old geometry to prevent leaks

### 1.3 Double Buffering
- **Build offline**: Prepare new geometry without affecting current render
- **Atomic swap**: Replace geometry reference in single operation
- **No visual glitches**: User never sees partial geometry

---

## 2. Chunk Mesh Manager

### 2.1 TypeScript Interface

```typescript
interface ChunkMeshData {
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  colors?: Float32Array;
  triangleCount: number;
  vertexCount: number;
  dataVersion: number;
}

interface ChunkRenderState {
  mesh: Mesh;
  geometry: BufferGeometry;
  currentVersion: number;
  isVisible: boolean;
}
```

### 2.2 Mesh Pool

```typescript
import {
  BufferAttribute,
  BufferGeometry,
  Mesh,
  MeshStandardMaterial,
  DoubleSide,
  Group,
} from 'three';

/**
 * Manages Three.js mesh objects for voxel chunks.
 * Handles creation, updates, and disposal.
 */
export class ChunkMeshPool {
  private chunks: Map<string, ChunkRenderState> = new Map();
  private group: Group;
  private material: MeshStandardMaterial;

  constructor(parentGroup: Group) {
    this.group = parentGroup;

    // Shared material for all chunks (or per-material-type if needed)
    this.material = new MeshStandardMaterial({
      color: 0x7ad8ff,
      roughness: 0.35,
      metalness: 0.1,
      vertexColors: false,
      side: DoubleSide,
    });
  }

  /**
   * Get chunk key from coordinates
   */
  private chunkKey(x: number, y: number, z: number): string {
    return `${x},${y},${z}`;
  }

  /**
   * Update or create mesh for a chunk
   */
  updateChunk(
    coord: { x: number; y: number; z: number },
    data: ChunkMeshData
  ): void {
    const key = this.chunkKey(coord.x, coord.y, coord.z);
    let state = this.chunks.get(key);

    if (!state) {
      // Create new mesh
      state = this.createChunkMesh(coord);
      this.chunks.set(key, state);
      this.group.add(state.mesh);
    }

    // Skip if data version hasn't changed
    if (state.currentVersion === data.dataVersion) {
      return;
    }

    // Build new geometry
    const newGeometry = this.buildGeometry(data);

    // Swap geometry (atomic operation)
    const oldGeometry = state.geometry;
    state.mesh.geometry = newGeometry;
    state.geometry = newGeometry;
    state.currentVersion = data.dataVersion;

    // Dispose old geometry
    oldGeometry.dispose();
  }

  /**
   * Create a new chunk mesh (called once per chunk)
   */
  private createChunkMesh(coord: { x: number; y: number; z: number }): ChunkRenderState {
    const geometry = new BufferGeometry();
    const mesh = new Mesh(geometry, this.material);

    // Set chunk position in world space
    // (Assuming chunk size and voxel size are known)
    mesh.position.set(
      coord.x * CHUNK_SIZE * VOXEL_SIZE,
      coord.y * CHUNK_SIZE * VOXEL_SIZE,
      coord.z * CHUNK_SIZE * VOXEL_SIZE
    );

    mesh.frustumCulled = true;
    mesh.name = `chunk_${coord.x}_${coord.y}_${coord.z}`;

    return {
      mesh,
      geometry,
      currentVersion: -1,
      isVisible: true,
    };
  }

  /**
   * Build BufferGeometry from mesh data
   */
  private buildGeometry(data: ChunkMeshData): BufferGeometry {
    const geometry = new BufferGeometry();

    // Position attribute (required)
    geometry.setAttribute(
      'position',
      new BufferAttribute(data.positions, 3)
    );

    // Normal attribute
    geometry.setAttribute(
      'normal',
      new BufferAttribute(data.normals, 3)
    );

    // Index buffer
    geometry.setIndex(new BufferAttribute(data.indices, 1));

    // Optional: vertex colors
    if (data.colors) {
      geometry.setAttribute(
        'color',
        new BufferAttribute(data.colors, 3)
      );
      // Enable vertex colors on material if not already
      if (!this.material.vertexColors) {
        this.material.vertexColors = true;
        this.material.needsUpdate = true;
      }
    }

    // Compute bounds for frustum culling
    geometry.computeBoundingBox();
    geometry.computeBoundingSphere();

    return geometry;
  }

  /**
   * Remove a chunk from the pool
   */
  removeChunk(coord: { x: number; y: number; z: number }): void {
    const key = this.chunkKey(coord.x, coord.y, coord.z);
    const state = this.chunks.get(key);

    if (state) {
      this.group.remove(state.mesh);
      state.geometry.dispose();
      this.chunks.delete(key);
    }
  }

  /**
   * Set visibility for a chunk
   */
  setChunkVisible(coord: { x: number; y: number; z: number }, visible: boolean): void {
    const key = this.chunkKey(coord.x, coord.y, coord.z);
    const state = this.chunks.get(key);

    if (state) {
      state.mesh.visible = visible;
      state.isVisible = visible;
    }
  }

  /**
   * Dispose all chunks and resources
   */
  dispose(): void {
    for (const state of this.chunks.values()) {
      this.group.remove(state.mesh);
      state.geometry.dispose();
    }
    this.chunks.clear();
    this.material.dispose();
  }

  /**
   * Get statistics
   */
  getStats(): { chunkCount: number; totalTriangles: number; totalVertices: number } {
    let totalTriangles = 0;
    let totalVertices = 0;

    for (const state of this.chunks.values()) {
      const geo = state.geometry;
      if (geo.index) {
        totalTriangles += geo.index.count / 3;
      }
      const pos = geo.getAttribute('position');
      if (pos) {
        totalVertices += pos.count;
      }
    }

    return {
      chunkCount: this.chunks.size,
      totalTriangles,
      totalVertices,
    };
  }
}

// Constants (must match Rust side - see greedy-mesh-implementation-plan.md)
const CHUNK_SIZE = 64;  // 64³ total, 62³ usable with 1-voxel padding
const VOXEL_SIZE = 0.1;
```

---

## 3. Preallocated Buffers with DrawRange

For frequently-updated chunks, preallocate larger buffers and use `drawRange`.

### 3.1 Preallocated Geometry

```typescript
/**
 * Create a preallocated geometry with capacity for dynamic updates
 */
function createPreallocatedGeometry(
  maxVertices: number,
  maxIndices: number
): BufferGeometry {
  const geometry = new BufferGeometry();

  // Preallocate position buffer
  const positions = new Float32Array(maxVertices * 3);
  const posAttr = new BufferAttribute(positions, 3);
  posAttr.setUsage(DynamicDrawUsage); // Hint for frequent updates
  geometry.setAttribute('position', posAttr);

  // Preallocate normal buffer
  const normals = new Float32Array(maxVertices * 3);
  const normAttr = new BufferAttribute(normals, 3);
  normAttr.setUsage(DynamicDrawUsage);
  geometry.setAttribute('normal', normAttr);

  // Preallocate index buffer
  const indices = new Uint32Array(maxIndices);
  const indexAttr = new BufferAttribute(indices, 1);
  indexAttr.setUsage(DynamicDrawUsage);
  geometry.setIndex(indexAttr);

  // Initially draw nothing
  geometry.setDrawRange(0, 0);

  return geometry;
}

/**
 * Update preallocated geometry with new data
 */
function updatePreallocatedGeometry(
  geometry: BufferGeometry,
  data: ChunkMeshData
): boolean {
  const posAttr = geometry.getAttribute('position') as BufferAttribute;
  const normAttr = geometry.getAttribute('normal') as BufferAttribute;
  const indexAttr = geometry.getIndex() as BufferAttribute;

  // Check capacity
  if (data.vertexCount > posAttr.count || data.indices.length > indexAttr.count) {
    // Exceeds capacity - need to reallocate
    return false;
  }

  // Copy data into existing buffers
  (posAttr.array as Float32Array).set(data.positions);
  (normAttr.array as Float32Array).set(data.normals);
  (indexAttr.array as Uint32Array).set(data.indices);

  // Mark for upload
  posAttr.needsUpdate = true;
  normAttr.needsUpdate = true;
  indexAttr.needsUpdate = true;

  // Update draw range to only render valid data
  geometry.setDrawRange(0, data.indices.length);

  // Update bounds
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();

  return true;
}
```

### 3.2 Capacity Calculation

```typescript
/**
 * Calculate buffer capacity for a chunk
 *
 * For 64³ chunks with binary meshing (62³ usable due to 1-voxel padding):
 *
 * Worst case: fully fragmented (checkerboard pattern)
 * - Max faces: 62³ * 6 / 2 = 714,216 faces
 * - Vertices per face: 4
 * - Indices per face: 6
 *
 * With greedy meshing, typical case is 10-100x smaller.
 */
function calculateChunkCapacity(chunkSize: number): {
  maxVertices: number;
  maxIndices: number;
} {
  // Use usable size (chunkSize - 2 for padding on each side)
  const usableSize = chunkSize - 2;

  // Conservative estimate: 25% of worst case
  const worstCaseFaces = (usableSize ** 3 * 6) / 2;
  const estimatedFaces = worstCaseFaces * 0.25;

  return {
    maxVertices: Math.ceil(estimatedFaces * 4),
    maxIndices: Math.ceil(estimatedFaces * 6),
  };
}

// For 64³ chunk (62³ usable):
// Worst case: 714,216 faces → 2,856,864 vertices, 4,285,296 indices
// 25% estimate: 178,554 faces → 714,216 vertices, 1,071,324 indices
// Memory: ~17 MB per chunk buffer (worst case)
//
// Typical terrain surface: 10-50 KB per chunk
// Complex caves: 50-200 KB per chunk
```

---

## 4. Double-Buffer Swap Protocol

### 4.1 Double-Buffered Chunk State

```typescript
interface DoubleBufferedChunk {
  mesh: Mesh;
  activeGeometry: BufferGeometry;
  pendingGeometry: BufferGeometry | null;
  pendingVersion: number;
  activeVersion: number;
}

class DoubleBufferedMeshPool {
  private chunks: Map<string, DoubleBufferedChunk> = new Map();

  /**
   * Submit new geometry for a chunk (doesn't swap yet)
   */
  submitGeometry(
    coord: { x: number; y: number; z: number },
    data: ChunkMeshData
  ): void {
    const key = this.chunkKey(coord.x, coord.y, coord.z);
    const chunk = this.chunks.get(key);

    if (!chunk) {
      // Create new chunk with immediate geometry
      this.createChunk(coord, data);
      return;
    }

    // Build pending geometry
    const pending = this.buildGeometry(data);

    // Dispose any existing pending geometry
    if (chunk.pendingGeometry) {
      chunk.pendingGeometry.dispose();
    }

    chunk.pendingGeometry = pending;
    chunk.pendingVersion = data.dataVersion;
  }

  /**
   * Swap all pending geometries to active
   * Call this once per frame, after all submits
   */
  swapAll(): number {
    let swapCount = 0;

    for (const chunk of this.chunks.values()) {
      if (chunk.pendingGeometry && chunk.pendingVersion > chunk.activeVersion) {
        // Swap
        const old = chunk.activeGeometry;
        chunk.mesh.geometry = chunk.pendingGeometry;
        chunk.activeGeometry = chunk.pendingGeometry;
        chunk.activeVersion = chunk.pendingVersion;
        chunk.pendingGeometry = null;

        // Dispose old
        old.dispose();
        swapCount++;
      }
    }

    return swapCount;
  }

  // ... other methods similar to ChunkMeshPool
}
```

---

## 5. Clipping Planes for Slicing

Use material clipping planes instead of rebuilding geometry for cross-section views.

### 5.1 Clipping Plane Setup

```typescript
import { Plane, Vector3 } from 'three';

/**
 * Manages clipping planes for cross-section visualization
 */
class SlicingManager {
  private planes: Plane[] = [];
  private enabled: boolean = false;

  constructor(private material: MeshStandardMaterial) {
    // Initialize with default planes (disabled)
    this.planes = [
      new Plane(new Vector3(1, 0, 0), 0),   // X plane
      new Plane(new Vector3(0, 1, 0), 0),   // Y plane
      new Plane(new Vector3(0, 0, 1), 0),   // Z plane
    ];
  }

  /**
   * Enable/disable slicing
   */
  setEnabled(enabled: boolean): void {
    this.enabled = enabled;
    this.updateMaterial();
  }

  /**
   * Set X-axis slice position
   */
  setSliceX(position: number, direction: 1 | -1 = 1): void {
    this.planes[0].normal.set(direction, 0, 0);
    this.planes[0].constant = -position * direction;
    this.updateMaterial();
  }

  /**
   * Set Y-axis slice position
   */
  setSliceY(position: number, direction: 1 | -1 = 1): void {
    this.planes[1].normal.set(0, direction, 0);
    this.planes[1].constant = -position * direction;
    this.updateMaterial();
  }

  /**
   * Set Z-axis slice position
   */
  setSliceZ(position: number, direction: 1 | -1 = 1): void {
    this.planes[2].normal.set(0, 0, direction);
    this.planes[2].constant = -position * direction;
    this.updateMaterial();
  }

  /**
   * Set arbitrary slice plane
   */
  setCustomPlane(normal: Vector3, distance: number): void {
    this.planes[0].normal.copy(normal).normalize();
    this.planes[0].constant = distance;
    this.updateMaterial();
  }

  private updateMaterial(): void {
    if (this.enabled) {
      this.material.clippingPlanes = this.planes;
      this.material.clipIntersection = false; // Clip outside all planes
    } else {
      this.material.clippingPlanes = null;
    }
    this.material.needsUpdate = true;
  }
}
```

### 5.2 Renderer Configuration

```typescript
// Enable clipping in renderer
renderer.localClippingEnabled = true;

// For WebGPU renderer
// (WebGPURenderer also supports localClippingEnabled)
```

---

## 6. Memory Management

### 6.1 Geometry Disposal

```typescript
/**
 * Properly dispose a BufferGeometry and all its attributes
 */
function disposeGeometry(geometry: BufferGeometry): void {
  // Dispose all attributes
  for (const key in geometry.attributes) {
    const attr = geometry.attributes[key];
    if (attr.array instanceof ArrayBuffer) {
      // ArrayBuffer will be GC'd
    }
  }

  // Dispose index
  if (geometry.index) {
    // Index array will be GC'd
  }

  // This releases GPU buffers
  geometry.dispose();
}
```

### 6.2 Memory Budget Tracking

```typescript
interface MemoryStats {
  geometryCount: number;
  totalVertexBytes: number;
  totalIndexBytes: number;
  totalBytes: number;
}

function calculateMemoryUsage(pool: ChunkMeshPool): MemoryStats {
  let geometryCount = 0;
  let totalVertexBytes = 0;
  let totalIndexBytes = 0;

  // Iterate through chunks and sum memory usage
  // (Implementation depends on pool internals)

  return {
    geometryCount,
    totalVertexBytes,
    totalIndexBytes,
    totalBytes: totalVertexBytes + totalIndexBytes,
  };
}

/**
 * Memory budget enforcement
 */
class MemoryBudget {
  constructor(private maxBytes: number) {}

  /**
   * Check if adding geometry would exceed budget
   */
  canAllocate(bytes: number, currentUsage: number): boolean {
    return currentUsage + bytes <= this.maxBytes;
  }

  /**
   * Get chunks to evict to make room (furthest from camera first)
   */
  getEvictionCandidates(
    chunks: Map<string, { coord: ChunkCoord; byteSize: number }>,
    cameraPos: Vector3,
    bytesNeeded: number
  ): ChunkCoord[] {
    // Sort by distance from camera (descending)
    const sorted = Array.from(chunks.entries())
      .map(([key, chunk]) => ({
        coord: chunk.coord,
        byteSize: chunk.byteSize,
        distance: this.distanceToCamera(chunk.coord, cameraPos),
      }))
      .sort((a, b) => b.distance - a.distance);

    // Collect chunks until we have enough bytes
    const evict: ChunkCoord[] = [];
    let freedBytes = 0;

    for (const chunk of sorted) {
      if (freedBytes >= bytesNeeded) break;
      evict.push(chunk.coord);
      freedBytes += chunk.byteSize;
    }

    return evict;
  }

  private distanceToCamera(coord: ChunkCoord, cameraPos: Vector3): number {
    // Calculate chunk center and return distance
    // ...
    return 0;
  }
}
```

---

## 7. WebGPU Compatibility

### 7.1 Renderer Detection

```typescript
/**
 * Check if using WebGPU renderer
 */
function isWebGPURenderer(renderer: WebGLRenderer | WebGPURenderer): boolean {
  return 'backend' in renderer && renderer.backend === 'webgpu';
}
```

### 7.2 Buffer Usage Hints

```typescript
import { StaticDrawUsage, DynamicDrawUsage, StreamDrawUsage } from 'three';

/**
 * Choose appropriate buffer usage based on update frequency
 */
function getBufferUsage(updateFrequency: 'static' | 'dynamic' | 'stream'): number {
  switch (updateFrequency) {
    case 'static':
      // Data set once, drawn many times
      return StaticDrawUsage;
    case 'dynamic':
      // Data updated occasionally
      return DynamicDrawUsage;
    case 'stream':
      // Data updated every frame
      return StreamDrawUsage;
  }
}

// For voxel chunks:
// - Static scenes: StaticDrawUsage
// - Editable voxels: DynamicDrawUsage
```

---

## 8. Debug Visualization

### 8.1 Chunk Bounds Visualization

```typescript
import { Box3Helper, Box3 } from 'three';

class ChunkDebugVisuals {
  private boundingBoxHelpers: Map<string, Box3Helper> = new Map();
  private debugGroup: Group;

  constructor(scene: Scene) {
    this.debugGroup = new Group();
    this.debugGroup.name = 'chunk-debug';
    this.debugGroup.visible = false;
    scene.add(this.debugGroup);
  }

  /**
   * Update bounding box helper for a chunk
   */
  updateChunkBounds(
    coord: { x: number; y: number; z: number },
    geometry: BufferGeometry
  ): void {
    const key = `${coord.x},${coord.y},${coord.z}`;

    if (!geometry.boundingBox) {
      geometry.computeBoundingBox();
    }

    let helper = this.boundingBoxHelpers.get(key);
    if (!helper) {
      helper = new Box3Helper(geometry.boundingBox!, 0x00ff00);
      this.boundingBoxHelpers.set(key, helper);
      this.debugGroup.add(helper);
    } else {
      helper.box.copy(geometry.boundingBox!);
    }
  }

  /**
   * Toggle debug visualization
   */
  setVisible(visible: boolean): void {
    this.debugGroup.visible = visible;
  }
}
```

### 8.2 Statistics Overlay

```typescript
interface RenderStats {
  fps: number;
  drawCalls: number;
  triangles: number;
  geometries: number;
  textures: number;
}

function getRenderStats(renderer: WebGLRenderer): RenderStats {
  const info = renderer.info;
  return {
    fps: 0, // Calculate externally
    drawCalls: info.render.calls,
    triangles: info.render.triangles,
    geometries: info.memory.geometries,
    textures: info.memory.textures,
  };
}
```

---

## Summary

| Component | Responsibility |
|-----------|----------------|
| `ChunkMeshPool` | Mesh lifecycle management, geometry swapping |
| `DoubleBufferedMeshPool` | Flicker-free geometry updates |
| `SlicingManager` | Clipping plane-based cross-sections |
| `MemoryBudget` | GPU memory tracking and eviction |
| `ChunkDebugVisuals` | Bounding box and stats visualization |

### Key Patterns

1. **Stable Mesh objects**: Create once, swap geometry
2. **Explicit disposal**: Always dispose old geometry
3. **DrawRange for dynamic data**: Preallocate, then limit visible range
4. **Double buffering**: Build new geometry before swapping
5. **Clipping planes**: Slice without geometry rebuild
