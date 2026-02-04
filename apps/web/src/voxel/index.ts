/**
 * Voxel chunk rendering system.
 *
 * This module provides chunk-based rendering for voxel data:
 * - ChunkMeshPool: Manages Three.js mesh objects with double buffering
 * - SlicingManager: GPU clipping planes for visualization
 * - ChunkRenderManager: High-level API coordinating everything
 *
 * @example
 * ```typescript
 * import { ChunkRenderManager } from "./voxel";
 *
 * // Create manager
 * const renderManager = new ChunkRenderManager({
 *   voxelSize: 0.1,
 *   chunkSize: 62,
 * });
 *
 * // Add to scene
 * scene.add(renderManager.group);
 *
 * // Configure renderer for clipping
 * renderManager.configureRenderer(renderer);
 *
 * // In update loop: set pending meshes from WASM
 * renderManager.setChunkMeshFromWasm(coord, wasmResult, version);
 *
 * // Swap all pending meshes at end of frame
 * renderManager.swapPendingMeshes();
 * ```
 */

// Types
export type {
  ChunkCoord,
  ChunkMeshData,
  ChunkMeshState,
  ChunkMeshPoolConfig,
  MeshPoolStats,
  SliceAxis,
  SliceConfig,
  SlicingConfig,
} from "./types";

export { chunkKey, parseChunkKey } from "./types";

// Classes
export { ChunkMeshPool } from "./ChunkMeshPool";
export { SlicingManager } from "./SlicingManager";
export { ChunkRenderManager } from "./ChunkRenderManager";
export type { ChunkRenderConfig, RenderStats } from "./ChunkRenderManager";
