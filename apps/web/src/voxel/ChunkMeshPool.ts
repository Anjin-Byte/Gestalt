/**
 * ChunkMeshPool - Manages Three.js mesh objects for voxel chunks.
 *
 * Features:
 * - Stable mesh object reuse (mesh objects stay in scene, geometry swaps)
 * - Double buffering for flicker-free updates
 * - Proper geometry disposal to prevent memory leaks
 * - Statistics tracking for debugging
 */

import {
  BufferAttribute,
  BufferGeometry,
  Mesh,
  Group,
  Material,
} from "three";
import type {
  ChunkCoord,
  ChunkMeshData,
  ChunkMeshState,
  ChunkMeshPoolConfig,
  MeshPoolStats,
} from "./types";
import { chunkKey, parseChunkKey } from "./types";

/**
 * Pool entry containing mesh state and Three.js objects.
 */
type PoolEntry = {
  coord: ChunkCoord;
  state: ChunkMeshState;
  /** Data version when last meshed (for staleness detection) */
  dataVersion: number;
};

/**
 * ChunkMeshPool manages Three.js mesh objects for voxel chunks.
 *
 * Design principles:
 * - Mesh objects are stable (created once, geometry swapped)
 * - Double buffering prevents visual flicker during updates
 * - Geometry is properly disposed to prevent WebGL memory leaks
 */
export class ChunkMeshPool {
  private pool: Map<string, PoolEntry> = new Map();
  private material: Material;
  private parent: Group;
  private voxelSize: number;
  private chunkSize: number;

  /** Geometries pending disposal (deferred to avoid mid-frame issues) */
  private disposalQueue: BufferGeometry[] = [];

  constructor(config: ChunkMeshPoolConfig) {
    this.material = config.material;
    this.parent = config.parent;
    this.voxelSize = config.voxelSize;
    this.chunkSize = config.chunkSize;
  }

  // ========================================================================
  // Chunk Management
  // ========================================================================

  /**
   * Get or create a pool entry for a chunk.
   */
  getOrCreate(coord: ChunkCoord): PoolEntry {
    const key = chunkKey(coord);
    let entry = this.pool.get(key);

    if (!entry) {
      entry = {
        coord,
        state: { status: "empty" },
        dataVersion: 0,
      };
      this.pool.set(key, entry);
    }

    return entry;
  }

  /**
   * Check if a chunk exists in the pool.
   */
  has(coord: ChunkCoord): boolean {
    return this.pool.has(chunkKey(coord));
  }

  /**
   * Remove a chunk from the pool and dispose its resources.
   */
  remove(coord: ChunkCoord): boolean {
    const key = chunkKey(coord);
    const entry = this.pool.get(key);

    if (!entry) {
      return false;
    }

    this.disposeEntry(entry);
    this.pool.delete(key);
    return true;
  }

  /**
   * Clear all chunks from the pool.
   */
  clear(): void {
    for (const entry of this.pool.values()) {
      this.disposeEntry(entry);
    }
    this.pool.clear();
    this.flushDisposalQueue();
  }

  // ========================================================================
  // Mesh Data Management
  // ========================================================================

  /**
   * Set pending mesh data for a chunk.
   *
   * The mesh won't be visible until swapPending() is called.
   * This enables double-buffering for flicker-free updates.
   */
  setPending(coord: ChunkCoord, data: ChunkMeshData, dataVersion: number): void {
    const entry = this.getOrCreate(coord);
    entry.dataVersion = dataVersion;

    switch (entry.state.status) {
      case "empty":
      case "pending":
        // Simple case: just set pending data
        entry.state = { status: "pending", data };
        break;

      case "ready":
        // Has active mesh, enter swapping state
        entry.state = {
          status: "swapping",
          pending: data,
          active: entry.state,
        };
        break;

      case "swapping":
        // Already swapping, update pending data
        entry.state.pending = data;
        break;
    }
  }

  /**
   * Swap pending mesh data into active geometry.
   *
   * Returns true if any swaps occurred.
   */
  swapPending(): boolean {
    let swapped = false;

    for (const entry of this.pool.values()) {
      if (this.swapEntryPending(entry)) {
        swapped = true;
      }
    }

    // Process deferred disposals
    this.flushDisposalQueue();

    return swapped;
  }

  /**
   * Swap pending data for a single chunk.
   */
  private swapEntryPending(entry: PoolEntry): boolean {
    switch (entry.state.status) {
      case "pending": {
        // Create new mesh and geometry
        const geometry = this.createGeometry(entry.state.data);
        const mesh = new Mesh(geometry, this.material);
        mesh.name = `chunk-${chunkKey(entry.coord)}`;

        // Position mesh in world space
        const origin = this.chunkOrigin(entry.coord);
        mesh.position.set(origin.x, origin.y, origin.z);

        // Add to scene
        this.parent.add(mesh);

        entry.state = { status: "ready", mesh, geometry };
        return true;
      }

      case "swapping": {
        // Dispose old geometry
        this.disposalQueue.push(entry.state.active.geometry);

        // Create new geometry and swap
        const geometry = this.createGeometry(entry.state.pending);
        const mesh = entry.state.active.mesh;

        // Swap geometry on existing mesh (stable object reference)
        mesh.geometry = geometry;

        entry.state = { status: "ready", mesh, geometry };
        return true;
      }

      default:
        return false;
    }
  }

  // ========================================================================
  // Geometry Creation
  // ========================================================================

  /**
   * Create BufferGeometry from WASM mesh data.
   */
  private createGeometry(data: ChunkMeshData): BufferGeometry {
    const geometry = new BufferGeometry();

    // Positions (required)
    geometry.setAttribute(
      "position",
      new BufferAttribute(this.ensureArrayBuffer(data.positions), 3)
    );

    // Normals (required)
    geometry.setAttribute(
      "normal",
      new BufferAttribute(this.ensureArrayBuffer(data.normals), 3)
    );

    // Indices (required)
    geometry.setIndex(
      new BufferAttribute(this.ensureUint32ArrayBuffer(data.indices), 1)
    );

    // UVs (optional)
    if (data.uvs && data.uvs.length > 0) {
      geometry.setAttribute(
        "uv",
        new BufferAttribute(this.ensureArrayBuffer(data.uvs), 2)
      );
    }

    // Material IDs as vertex attribute (optional, for texture atlas)
    if (data.materialIds && data.materialIds.length > 0) {
      geometry.setAttribute(
        "materialId",
        new BufferAttribute(this.ensureUint16ArrayBuffer(data.materialIds), 1)
      );
    }

    // Compute bounding volumes for frustum culling
    geometry.computeBoundingBox();
    geometry.computeBoundingSphere();

    return geometry;
  }

  /**
   * Ensure Float32Array has proper ArrayBuffer (not SharedArrayBuffer).
   */
  private ensureArrayBuffer(input: Float32Array): Float32Array {
    if (input.buffer instanceof ArrayBuffer) {
      return input;
    }
    return new Float32Array(input);
  }

  /**
   * Ensure Uint32Array has proper ArrayBuffer.
   */
  private ensureUint32ArrayBuffer(input: Uint32Array): Uint32Array {
    if (input.buffer instanceof ArrayBuffer) {
      return input;
    }
    return new Uint32Array(input);
  }

  /**
   * Ensure Uint16Array has proper ArrayBuffer.
   */
  private ensureUint16ArrayBuffer(input: Uint16Array): Uint16Array {
    if (input.buffer instanceof ArrayBuffer) {
      return input;
    }
    return new Uint16Array(input);
  }

  // ========================================================================
  // World Positioning
  // ========================================================================

  /**
   * Calculate world-space origin for a chunk.
   */
  private chunkOrigin(coord: ChunkCoord): { x: number; y: number; z: number } {
    const size = this.chunkSize * this.voxelSize;
    return {
      x: coord.x * size,
      y: coord.y * size,
      z: coord.z * size,
    };
  }

  // ========================================================================
  // Disposal
  // ========================================================================

  /**
   * Dispose a pool entry's resources.
   */
  private disposeEntry(entry: PoolEntry): void {
    switch (entry.state.status) {
      case "ready":
        this.parent.remove(entry.state.mesh);
        this.disposalQueue.push(entry.state.geometry);
        break;

      case "swapping":
        this.parent.remove(entry.state.active.mesh);
        this.disposalQueue.push(entry.state.active.geometry);
        break;
    }
  }

  /**
   * Process deferred geometry disposals.
   */
  private flushDisposalQueue(): void {
    for (const geometry of this.disposalQueue) {
      geometry.dispose();
    }
    this.disposalQueue = [];
  }

  /**
   * Dispose all resources and clear the pool.
   */
  dispose(): void {
    this.clear();
  }

  // ========================================================================
  // Statistics
  // ========================================================================

  /**
   * Get pool statistics for debugging.
   */
  getStats(): MeshPoolStats {
    let chunksWithMesh = 0;
    let pendingSwaps = 0;
    let triangleCount = 0;
    let vertexCount = 0;
    let geometryCount = 0;

    for (const entry of this.pool.values()) {
      switch (entry.state.status) {
        case "pending":
          pendingSwaps++;
          break;

        case "ready":
          chunksWithMesh++;
          geometryCount++;
          triangleCount += entry.state.geometry.index
            ? entry.state.geometry.index.count / 3
            : 0;
          vertexCount += entry.state.geometry.attributes.position?.count ?? 0;
          break;

        case "swapping":
          chunksWithMesh++;
          pendingSwaps++;
          geometryCount++;
          triangleCount += entry.state.active.geometry.index
            ? entry.state.active.geometry.index.count / 3
            : 0;
          vertexCount += entry.state.active.geometry.attributes.position?.count ?? 0;
          break;
      }
    }

    return {
      totalChunks: this.pool.size,
      chunksWithMesh,
      pendingSwaps,
      triangleCount,
      vertexCount,
      geometryCount,
    };
  }

  /**
   * Get all chunk coordinates in the pool.
   */
  getChunkCoords(): ChunkCoord[] {
    return Array.from(this.pool.keys()).map(parseChunkKey);
  }

  /**
   * Get the mesh for a chunk (if ready).
   */
  getMesh(coord: ChunkCoord): Mesh | null {
    const entry = this.pool.get(chunkKey(coord));
    if (!entry) return null;

    switch (entry.state.status) {
      case "ready":
        return entry.state.mesh;
      case "swapping":
        return entry.state.active.mesh;
      default:
        return null;
    }
  }

  /**
   * Update material for all meshes.
   */
  setMaterial(material: Material): void {
    this.material = material;

    for (const entry of this.pool.values()) {
      const mesh = this.getMeshFromEntry(entry);
      if (mesh) {
        mesh.material = material;
      }
    }
  }

  private getMeshFromEntry(entry: PoolEntry): Mesh | null {
    switch (entry.state.status) {
      case "ready":
        return entry.state.mesh;
      case "swapping":
        return entry.state.active.mesh;
      default:
        return null;
    }
  }
}
