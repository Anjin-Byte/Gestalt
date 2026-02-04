/**
 * ChunkRenderManager - High-level manager for chunk-based voxel rendering.
 *
 * Coordinates:
 * - ChunkMeshPool for mesh management
 * - SlicingManager for clipping planes
 * - Integration with WASM mesher
 *
 * This is the main entry point for the chunk rendering system.
 */

import {
  Group,
  MeshStandardMaterial,
  Material,
  DoubleSide,
  WebGLRenderer,
} from "three";
import { ChunkMeshPool } from "./ChunkMeshPool";
import { SlicingManager } from "./SlicingManager";
import type { MeshResult } from "../wasm/wasm_greedy_mesher/wasm_greedy_mesher";
import type {
  ChunkCoord,
  ChunkMeshData,
  MeshPoolStats,
  SliceAxis,
  SlicingConfig,
} from "./types";

/**
 * Configuration for ChunkRenderManager.
 */
export type ChunkRenderConfig = {
  /** Voxel size in world units */
  voxelSize: number;
  /** Usable chunk size (typically 62) */
  chunkSize: number;
  /** Optional custom material */
  material?: Material;
  /** Enable local clipping (required for slicing) */
  enableClipping?: boolean;
};

/**
 * Render statistics.
 */
export type RenderStats = MeshPoolStats & {
  slicingEnabled: boolean;
  activeSlices: number;
};

/**
 * ChunkRenderManager provides high-level chunk rendering management.
 */
export class ChunkRenderManager {
  /** Root group containing all chunk meshes */
  readonly group: Group;

  /** Mesh pool for chunk geometry */
  private meshPool: ChunkMeshPool;

  /** Slicing manager for clipping planes */
  private slicing: SlicingManager;

  /** Default material for chunks */
  private material: Material;

  /** Configuration */
  private config: ChunkRenderConfig;

  constructor(config: ChunkRenderConfig) {
    this.config = config;
    this.group = new Group();
    this.group.name = "chunk-render-root";

    // Create default material
    this.material = config.material ?? this.createDefaultMaterial();

    // Initialize slicing manager
    this.slicing = new SlicingManager();
    this.slicing.addMaterial(this.material);

    // Initialize mesh pool
    this.meshPool = new ChunkMeshPool({
      voxelSize: config.voxelSize,
      chunkSize: config.chunkSize,
      material: this.material,
      parent: this.group,
    });
  }

  /**
   * Create default material for voxel meshes.
   */
  private createDefaultMaterial(): Material {
    return new MeshStandardMaterial({
      color: 0x7ad8ff,
      roughness: 0.6,
      metalness: 0.1,
      side: DoubleSide,
    });
  }

  // ========================================================================
  // Renderer Configuration
  // ========================================================================

  /**
   * Configure renderer for clipping plane support.
   *
   * Call this once during setup if you want to use slicing.
   */
  configureRenderer(renderer: WebGLRenderer): void {
    renderer.localClippingEnabled = true;
  }

  // ========================================================================
  // Chunk Mesh Management
  // ========================================================================

  /**
   * Set pending mesh data for a chunk from WASM MeshResult.
   */
  setChunkMeshFromWasm(
    coord: ChunkCoord,
    result: MeshResult,
    dataVersion: number
  ): void {
    if (result.is_empty) {
      // Empty mesh - remove chunk if it exists
      this.meshPool.remove(coord);
      return;
    }

    const data: ChunkMeshData = {
      positions: new Float32Array(result.positions),
      normals: new Float32Array(result.normals),
      indices: new Uint32Array(result.indices),
      uvs: new Float32Array(result.uvs),
      materialIds: new Uint16Array(result.material_ids),
      triangleCount: result.triangle_count,
      vertexCount: result.vertex_count,
    };

    this.meshPool.setPending(coord, data, dataVersion);
  }

  /**
   * Set pending mesh data for a chunk directly.
   */
  setChunkMesh(
    coord: ChunkCoord,
    data: ChunkMeshData,
    dataVersion: number
  ): void {
    if (data.vertexCount === 0) {
      this.meshPool.remove(coord);
      return;
    }

    this.meshPool.setPending(coord, data, dataVersion);
  }

  /**
   * Swap all pending meshes into active geometry.
   *
   * Call this once per frame after setting all pending meshes.
   * Returns true if any swaps occurred.
   */
  swapPendingMeshes(): boolean {
    return this.meshPool.swapPending();
  }

  /**
   * Remove a chunk from rendering.
   */
  removeChunk(coord: ChunkCoord): boolean {
    return this.meshPool.remove(coord);
  }

  /**
   * Check if a chunk has a mesh.
   */
  hasChunk(coord: ChunkCoord): boolean {
    return this.meshPool.has(coord);
  }

  /**
   * Get all chunk coordinates currently being rendered.
   */
  getChunkCoords(): ChunkCoord[] {
    return this.meshPool.getChunkCoords();
  }

  // ========================================================================
  // Slicing
  // ========================================================================

  /**
   * Enable or disable slicing globally.
   */
  setSlicingEnabled(enabled: boolean): void {
    this.slicing.setEnabled(enabled);
  }

  /**
   * Check if slicing is enabled.
   */
  isSlicingEnabled(): boolean {
    return this.slicing.isEnabled();
  }

  /**
   * Set the slicing configuration.
   */
  setSlicingConfig(config: Partial<SlicingConfig>): void {
    this.slicing.setConfig(config);
  }

  /**
   * Get the slicing configuration.
   */
  getSlicingConfig(): SlicingConfig {
    return this.slicing.getConfig();
  }

  /**
   * Set slice position for an axis.
   */
  setSlicePosition(axis: SliceAxis, position: number): void {
    this.slicing.setSlicePosition(axis, position);
  }

  /**
   * Get slice position for an axis.
   */
  getSlicePosition(axis: SliceAxis): number {
    return this.slicing.getSlicePosition(axis);
  }

  /**
   * Enable or disable a slice axis.
   */
  setSliceEnabled(axis: SliceAxis, enabled: boolean): void {
    this.slicing.setSliceEnabled(axis, enabled);
  }

  /**
   * Set slice direction for an axis.
   */
  setSliceDirection(axis: SliceAxis, direction: 1 | -1): void {
    this.slicing.setSliceDirection(axis, direction);
  }

  // ========================================================================
  // Material
  // ========================================================================

  /**
   * Set the material for all chunk meshes.
   */
  setMaterial(material: Material): void {
    // Remove old material from slicing
    this.slicing.removeMaterial(this.material);

    // Update material
    this.material = material;
    this.meshPool.setMaterial(material);

    // Add new material to slicing
    this.slicing.addMaterial(material);
  }

  /**
   * Get the current material.
   */
  getMaterial(): Material {
    return this.material;
  }

  // ========================================================================
  // Statistics
  // ========================================================================

  /**
   * Get render statistics.
   */
  getStats(): RenderStats {
    const poolStats = this.meshPool.getStats();
    const slicingConfig = this.slicing.getConfig();

    let activeSlices = 0;
    if (slicingConfig.slices.x?.enabled) activeSlices++;
    if (slicingConfig.slices.y?.enabled) activeSlices++;
    if (slicingConfig.slices.z?.enabled) activeSlices++;

    return {
      ...poolStats,
      slicingEnabled: slicingConfig.enabled,
      activeSlices,
    };
  }

  // ========================================================================
  // Lifecycle
  // ========================================================================

  /**
   * Clear all chunks.
   */
  clear(): void {
    this.meshPool.clear();
  }

  /**
   * Dispose all resources.
   */
  dispose(): void {
    this.meshPool.dispose();
    this.slicing.dispose();

    // Dispose material if we created it
    if (!this.config.material) {
      this.material.dispose();
    }
  }
}
