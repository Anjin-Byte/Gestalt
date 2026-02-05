/**
 * Types for the voxel chunk rendering system.
 */

import type { BufferGeometry, Material, Mesh, Group } from "three";

/** Chunk coordinate in chunk-space */
export type ChunkCoord = {
  x: number;
  y: number;
  z: number;
};

/** Convert chunk coord to string key for Map storage */
export const chunkKey = (coord: ChunkCoord): string =>
  `${coord.x},${coord.y},${coord.z}`;

/** Parse chunk key back to coordinate */
export const parseChunkKey = (key: string): ChunkCoord => {
  const [x, y, z] = key.split(",").map(Number);
  return { x, y, z };
};

/** Mesh data from WASM mesher */
export type ChunkMeshData = {
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  uvs: Float32Array;
  materialIds: Uint16Array;
  triangleCount: number;
  vertexCount: number;
};

/** State of a chunk's mesh */
export type ChunkMeshState =
  | { status: "empty" }
  | { status: "pending"; data: ChunkMeshData }
  | { status: "ready"; mesh: Mesh; geometry: BufferGeometry }
  | { status: "swapping"; pending: ChunkMeshData; active: { mesh: Mesh; geometry: BufferGeometry } };

/** Statistics for mesh pool */
export type MeshPoolStats = {
  totalChunks: number;
  chunksWithMesh: number;
  pendingSwaps: number;
  triangleCount: number;
  vertexCount: number;
  geometryCount: number;
  /** Estimated GPU memory usage in bytes across all active geometries. */
  gpuMemoryBytes: number;
};

/** Memory budget configuration (mirrors Rust MemoryBudget) */
export type MemoryBudgetConfig = {
  maxBytes: number;
  highWatermark: number;
  lowWatermark: number;
  minChunks: number;
};

/** Clipping plane axis */
export type SliceAxis = "x" | "y" | "z";

/** Slice configuration */
export type SliceConfig = {
  axis: SliceAxis;
  position: number;
  direction: 1 | -1; // 1 = clip above, -1 = clip below
  enabled: boolean;
};

/** Configuration for the chunk mesh pool */
export type ChunkMeshPoolConfig = {
  /** Voxel size in world units */
  voxelSize: number;
  /** Usable chunk size (typically 62) */
  chunkSize: number;
  /** Material to use for meshes */
  material: Material;
  /** Parent group for all chunk meshes */
  parent: Group;
};

/** Configuration for slicing manager */
export type SlicingConfig = {
  /** Enable slicing */
  enabled: boolean;
  /** Slices for each axis */
  slices: {
    x?: SliceConfig;
    y?: SliceConfig;
    z?: SliceConfig;
  };
};
