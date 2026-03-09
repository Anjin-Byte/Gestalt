/**
 * Integration bridge: OBJ → GPU voxelize → ChunkManager ingest → greedy mesh.
 *
 * Orchestrates the ADR-0009 pipeline:
 * 1. GPU voxelizer produces CompactVoxel data (vx, vy, vz, material)
 * 2. ChunkManager ingests compact voxels into chunk system
 * 3. Frame update triggers greedy meshing → returns mesh outputs
 */

import type { ModuleOutput } from "../types";
import type { ChunkManagerClient } from "./workers/chunkManagerClient";
import { CS, generateBoxWireframe } from "./helpers";

/** Options for voxelizeToChunks. */
export type VoxelizeToChunksOpts = {
  /** Flat Float32Array of vertex positions (x, y, z triples). */
  positions: Float32Array;
  /** Uint32Array of triangle indices. */
  indices: Uint32Array;
  /** Per-triangle u16 material IDs (from buildMaterialTable). */
  materialTable: Uint16Array;
  /** World-space origin (x, y, z). */
  origin: [number, number, number];
  /** Voxel size in world units. */
  voxelSize: number;
  /** Grid dimensions [x, y, z]. */
  dims: [number, number, number];
  /** Epsilon for voxel-triangle intersection. */
  epsilon?: number;
  /** Show chunk boundary wireframes. */
  debugChunkBounds?: boolean;
};

/**
 * Pack a Uint16Array of material IDs into Uint32Array (two per word).
 *
 * Element `i` goes into word `i >> 1`, low 16 bits for even indices,
 * high 16 bits for odd indices. This matches the GPU shader's unpacking:
 * `word_idx = tri >> 1; shift = (tri & 1) << 4`.
 */
export const packMaterialTable = (table: Uint16Array): Uint32Array => {
  const wordCount = (table.length + 1) >>> 1;
  const packed = new Uint32Array(wordCount);
  for (let i = 0; i < table.length; i++) {
    const wordIdx = i >>> 1;
    const shift = (i & 1) << 4;
    packed[wordIdx] |= (table[i] & 0xffff) << shift;
  }
  return packed;
};

/**
 * Compute the global voxel-space origin from world origin and voxel size.
 * g_origin = floor(origin / voxelSize)
 */
const computeGOrigin = (
  origin: [number, number, number],
  voxelSize: number
): [number, number, number] => [
  Math.floor(origin[0] / voxelSize),
  Math.floor(origin[1] / voxelSize),
  Math.floor(origin[2] / voxelSize),
];

/**
 * Run the full OBJ → voxelize → chunk → mesh pipeline.
 *
 * Requires both a WasmVoxelizer instance (for GPU compact) and a
 * ChunkManagerClient (for ingestion + meshing).
 *
 * @param voxelizer - WasmVoxelizer instance (from wasm_voxelizer)
 * @param chunkManagerClient - Initialized ChunkManagerClient
 * @param opts - Pipeline options
 * @returns ModuleOutput array with mesh and optional debug outputs
 */
export const voxelizeToChunks = async (
  voxelizer: {
    voxelize_compact_voxels(
      positions: Float32Array,
      indices: Uint32Array,
      materialTable: Uint32Array,
      origin: Float32Array,
      voxelSize: number,
      dims: Uint32Array,
      epsilon: number,
      gOriginX: number,
      gOriginY: number,
      gOriginZ: number,
    ): Promise<{ voxels: Int32Array; count: number }>;
  },
  chunkManagerClient: ChunkManagerClient,
  opts: VoxelizeToChunksOpts
): Promise<ModuleOutput[]> => {
  const epsilon = opts.epsilon ?? 1e-4;
  const gOrigin = computeGOrigin(opts.origin, opts.voxelSize);
  const packedTable = packMaterialTable(opts.materialTable);

  // Step 1: GPU voxelize + compact
  const originArr = new Float32Array(opts.origin);
  const dimsArr = new Uint32Array(opts.dims);

  const compactResult = await voxelizer.voxelize_compact_voxels(
    opts.positions,
    opts.indices,
    packedTable,
    originArr,
    opts.voxelSize,
    dimsArr,
    epsilon,
    gOrigin[0],
    gOrigin[1],
    gOrigin[2],
  );

  if (compactResult.count === 0) {
    return [];
  }

  // Step 2: Ingest into ChunkManager
  await chunkManagerClient.ingestCompactVoxels(compactResult.voxels);

  // Step 3: Rebuild all dirty chunks and get meshes
  // Uses rebuildAllDirty instead of update() because std::time::Instant
  // is not available on wasm32-unknown-unknown.
  const rebuildResult = await chunkManagerClient.rebuildAllDirty();

  // Step 4: Convert to ModuleOutput
  const outputs: ModuleOutput[] = [];

  for (const mesh of rebuildResult.swappedMeshes) {
    outputs.push({
      kind: "mesh",
      mesh: {
        positions: mesh.positions,
        indices: mesh.indices,
        normals: mesh.normals,
      },
      label: `Chunk (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
    });
  }

  // Optional: debug chunk bounds
  if (opts.debugChunkBounds) {
    for (const mesh of rebuildResult.swappedMeshes) {
      const offsetX = mesh.coord.x * CS * opts.voxelSize;
      const offsetY = mesh.coord.y * CS * opts.voxelSize;
      const offsetZ = mesh.coord.z * CS * opts.voxelSize;
      const chunkExtent = CS * opts.voxelSize;

      const boundsPositions = generateBoxWireframe(
        offsetX,
        offsetY,
        offsetZ,
        offsetX + chunkExtent,
        offsetY + chunkExtent,
        offsetZ + chunkExtent,
      );
      outputs.push({
        kind: "lines",
        lines: { positions: boundsPositions, color: [0.0, 1.0, 1.0] },
        label: `Bounds (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
      });
    }
  }

  return outputs;
};
