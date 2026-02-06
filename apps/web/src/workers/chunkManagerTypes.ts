/**
 * Message protocol types for the chunk manager web worker.
 *
 * These extend the existing mesher worker with multi-chunk management
 * capabilities via the Rust ChunkManager exposed through WASM.
 */

// =========================================================================
// Configuration Types
// =========================================================================

/** Rebuild configuration (mirrors Rust RebuildConfig). */
export type RebuildConfig = {
  maxChunksPerFrame: number;
  maxTimeMs: number;
  voxelSize: number;
};

/** Memory budget configuration (mirrors Rust MemoryBudget). */
export type MemoryBudgetConfig = {
  maxBytes: number;
  highWatermark: number;
  lowWatermark: number;
  minChunks: number;
};

/** Chunk coordinate in chunk-space. */
export type ChunkCoord = {
  x: number;
  y: number;
  z: number;
};

// =========================================================================
// Frame Statistics
// =========================================================================

/** Statistics from a frame update (mirrors flattened FrameStats). */
export type FrameStats = {
  chunksRebuilt: number;
  trianglesGenerated: number;
  verticesGenerated: number;
  rebuildElapsedMs: number;
  queueRemaining: number;
  timeBudgetExceeded: boolean;
  chunkLimitReached: boolean;
  meshesSwapped: number;
  meshesDisposed: number;
  versionConflicts: number;
  chunksEvicted: number;
  bytesFreed: number;
  totalChunks: number;
  chunksWithMesh: number;
  dirtyChunks: number;
};

/** Debug info from chunk manager (mirrors ChunkDebugInfo). */
export type ChunkDebugInfo = {
  totalChunks: number;
  cleanChunks: number;
  dirtyChunks: number;
  meshingChunks: number;
  readyToSwapChunks: number;
  queueSize: number;
  totalTriangles: number;
  totalVertices: number;
  voxelMemoryBytes: number;
  meshMemoryBytes: number;
  budgetMaxBytes: number;
  budgetUsagePercent: number;
  budgetExceeded: boolean;
};

// =========================================================================
// Mesh Data for Transfer
// =========================================================================

/** Chunk mesh data transferred from worker to main thread. */
export type ChunkMeshTransfer = {
  coord: ChunkCoord;
  dataVersion: number;
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  uvs: Float32Array;
  materialIds: Uint16Array;
  vertexCount: number;
  triangleCount: number;
};

// =========================================================================
// Worker Messages: Main Thread -> Worker
// =========================================================================

export type ChunkManagerRequest =
  | { type: "cm-init"; config?: RebuildConfig; budget?: MemoryBudgetConfig }
  | { type: "cm-set-voxel"; wx: number; wy: number; wz: number; material: number }
  | { type: "cm-set-voxel-at"; vx: number; vy: number; vz: number; material: number }
  | { type: "cm-set-voxels-batch"; edits: Float32Array }
  | { type: "cm-get-voxel"; wx: number; wy: number; wz: number; requestId: number }
  | { type: "cm-update"; camX: number; camY: number; camZ: number }
  | { type: "cm-set-budget"; budget: MemoryBudgetConfig }
  | { type: "cm-touch-chunk"; cx: number; cy: number; cz: number }
  | { type: "cm-remove-chunk"; cx: number; cy: number; cz: number }
  | { type: "cm-clear" }
  | { type: "cm-debug-info" }
  | {
      type: "cm-generate-and-populate";
      gridSize: number;
      voxelSize: number;
      pattern: "solid" | "checkerboard" | "sphere" | "noise" | "perlin" | "simplex";
      simplexScale?: number;
      simplexOctaves?: number;
      simplexThreshold?: number;
    };

// =========================================================================
// Worker Messages: Worker -> Main Thread
// =========================================================================

export type ChunkManagerResponse =
  | { type: "cm-init-done"; voxelSize: number }
  | { type: "cm-init-error"; error: string }
  | {
      type: "cm-update-done";
      stats: FrameStats;
      swappedMeshes: ChunkMeshTransfer[];
      evictedCoords: ChunkCoord[];
    }
  | { type: "cm-voxel-result"; requestId: number; material: number }
  | { type: "cm-debug-info-result"; info: ChunkDebugInfo }
  | {
      type: "cm-populate-done";
      chunksRebuilt: number;
      swappedMeshes: ChunkMeshTransfer[];
      genTime: number;
      meshTime: number;
    }
  | { type: "cm-error"; error: string };
