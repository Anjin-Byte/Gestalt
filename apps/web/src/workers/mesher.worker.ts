/// <reference lib="webworker" />

/**
 * Mesher web worker.
 *
 * Loads the WASM greedy mesher and performs voxel generation + meshing
 * off the main thread. Results are transferred back via postMessage
 * using transferable buffers (zero-copy).
 */

import type {
  MesherRequest,
  MesherResponse,
  MeshJobParams,
  MeshJobResult,
  VoxelPattern,
} from "./mesherTypes";
import type {
  ChunkManagerRequest,
  ChunkManagerResponse,
  ChunkMeshTransfer,
  FrameStats,
  ChunkDebugInfo,
  ChunkCoord,
} from "./chunkManagerTypes";

declare const self: DedicatedWorkerGlobalScope;

type WasmModule = typeof import("../wasm/wasm_greedy_mesher/wasm_greedy_mesher");

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type WasmChunkManagerInstance = any;

let wasmModule: WasmModule | null = null;
let currentJobId: number | null = null;
let chunkManager: WasmChunkManagerInstance = null;

// ---------------------------------------------------------------------------
// Voxel generation (moved from wasmGreedyMesher module)
// ---------------------------------------------------------------------------

const PERLIN_SCALE = 0.12;
const PERLIN_OCTAVES = 3;
const PERLIN_SEED = 1337;
const SIMPLEX_SCALE = 0.08;
const SIMPLEX_OCTAVES = 3;
const SIMPLEX_THRESHOLD = 0;

const buildPerlinPermutation = (seed: number): Uint8Array => {
  const perm = new Uint8Array(256);
  for (let i = 0; i < 256; i++) perm[i] = i;

  let state = seed >>> 0;
  const rand = (): number => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x100000000;
  };

  for (let i = 255; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    const tmp = perm[i];
    perm[i] = perm[j];
    perm[j] = tmp;
  }

  const table = new Uint8Array(512);
  for (let i = 0; i < 512; i++) table[i] = perm[i & 255];
  return table;
};

const PERLIN_PERM = buildPerlinPermutation(PERLIN_SEED);

const fade = (t: number): number => t * t * t * (t * (t * 6 - 15) + 10);
const lerp = (a: number, b: number, t: number): number => a + t * (b - a);

const grad = (hash: number, x: number, y: number, z: number): number => {
  const h = hash & 15;
  const u = h < 8 ? x : y;
  const v = h < 4 ? y : (h === 12 || h === 14 ? x : z);
  return ((h & 1) === 0 ? u : -u) + ((h & 2) === 0 ? v : -v);
};

const perlin3 = (x: number, y: number, z: number): number => {
  const X = Math.floor(x) & 255;
  const Y = Math.floor(y) & 255;
  const Z = Math.floor(z) & 255;

  const xf = x - Math.floor(x);
  const yf = y - Math.floor(y);
  const zf = z - Math.floor(z);

  const u = fade(xf);
  const v = fade(yf);
  const w = fade(zf);

  const A = PERLIN_PERM[X] + Y;
  const AA = PERLIN_PERM[A] + Z;
  const AB = PERLIN_PERM[A + 1] + Z;
  const B = PERLIN_PERM[X + 1] + Y;
  const BA = PERLIN_PERM[B] + Z;
  const BB = PERLIN_PERM[B + 1] + Z;

  return lerp(
    lerp(
      lerp(grad(PERLIN_PERM[AA], xf, yf, zf), grad(PERLIN_PERM[BA], xf - 1, yf, zf), u),
      lerp(grad(PERLIN_PERM[AB], xf, yf - 1, zf), grad(PERLIN_PERM[BB], xf - 1, yf - 1, zf), u),
      v,
    ),
    lerp(
      lerp(grad(PERLIN_PERM[AA + 1], xf, yf, zf - 1), grad(PERLIN_PERM[BA + 1], xf - 1, yf, zf - 1), u),
      lerp(grad(PERLIN_PERM[AB + 1], xf, yf - 1, zf - 1), grad(PERLIN_PERM[BB + 1], xf - 1, yf - 1, zf - 1), u),
      v,
    ),
    w,
  );
};

const perlinFbm = (x: number, y: number, z: number): number => {
  let amplitude = 1;
  let frequency = 1;
  let total = 0;
  let max = 0;

  for (let i = 0; i < PERLIN_OCTAVES; i++) {
    total += perlin3(x * frequency, y * frequency, z * frequency) * amplitude;
    max += amplitude;
    amplitude *= 0.5;
    frequency *= 2;
  }

  return total / max;
};

const SIMPLEX_F3 = 1 / 3;
const SIMPLEX_G3 = 1 / 6;
const SIMPLEX_GRADS = [
  [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
  [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
  [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
];

const simplex3 = (x: number, y: number, z: number): number => {
  const s = (x + y + z) * SIMPLEX_F3;
  const i = Math.floor(x + s);
  const j = Math.floor(y + s);
  const k = Math.floor(z + s);

  const t = (i + j + k) * SIMPLEX_G3;
  const x0 = x - i + t;
  const y0 = y - j + t;
  const z0 = z - k + t;

  let i1 = 0, j1 = 0, k1 = 0;
  let i2 = 0, j2 = 0, k2 = 0;

  if (x0 >= y0) {
    if (y0 >= z0) { i1 = 1; j1 = 0; k1 = 0; i2 = 1; j2 = 1; k2 = 0; }
    else if (x0 >= z0) { i1 = 1; j1 = 0; k1 = 0; i2 = 1; j2 = 0; k2 = 1; }
    else { i1 = 0; j1 = 0; k1 = 1; i2 = 1; j2 = 0; k2 = 1; }
  } else {
    if (y0 < z0) { i1 = 0; j1 = 0; k1 = 1; i2 = 0; j2 = 1; k2 = 1; }
    else if (x0 < z0) { i1 = 0; j1 = 1; k1 = 0; i2 = 0; j2 = 1; k2 = 1; }
    else { i1 = 0; j1 = 1; k1 = 0; i2 = 1; j2 = 1; k2 = 0; }
  }

  const x1 = x0 - i1 + SIMPLEX_G3;
  const y1 = y0 - j1 + SIMPLEX_G3;
  const z1 = z0 - k1 + SIMPLEX_G3;
  const x2 = x0 - i2 + 2 * SIMPLEX_G3;
  const y2 = y0 - j2 + 2 * SIMPLEX_G3;
  const z2 = z0 - k2 + 2 * SIMPLEX_G3;
  const x3 = x0 - 1 + 3 * SIMPLEX_G3;
  const y3 = y0 - 1 + 3 * SIMPLEX_G3;
  const z3 = z0 - 1 + 3 * SIMPLEX_G3;

  const ii = i & 255;
  const jj = j & 255;
  const kk = k & 255;

  let n0 = 0, n1 = 0, n2 = 0, n3 = 0;

  let t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0;
  if (t0 > 0) {
    t0 *= t0;
    const gi0 = SIMPLEX_GRADS[PERLIN_PERM[ii + PERLIN_PERM[jj + PERLIN_PERM[kk]]] % 12];
    n0 = t0 * t0 * (gi0[0] * x0 + gi0[1] * y0 + gi0[2] * z0);
  }

  let t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1;
  if (t1 > 0) {
    t1 *= t1;
    const gi1 = SIMPLEX_GRADS[PERLIN_PERM[ii + i1 + PERLIN_PERM[jj + j1 + PERLIN_PERM[kk + k1]]] % 12];
    n1 = t1 * t1 * (gi1[0] * x1 + gi1[1] * y1 + gi1[2] * z1);
  }

  let t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2;
  if (t2 > 0) {
    t2 *= t2;
    const gi2 = SIMPLEX_GRADS[PERLIN_PERM[ii + i2 + PERLIN_PERM[jj + j2 + PERLIN_PERM[kk + k2]]] % 12];
    n2 = t2 * t2 * (gi2[0] * x2 + gi2[1] * y2 + gi2[2] * z2);
  }

  let t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3;
  if (t3 > 0) {
    t3 *= t3;
    const gi3 = SIMPLEX_GRADS[PERLIN_PERM[ii + 1 + PERLIN_PERM[jj + 1 + PERLIN_PERM[kk + 1]]] % 12];
    n3 = t3 * t3 * (gi3[0] * x3 + gi3[1] * y3 + gi3[2] * z3);
  }

  return 32 * (n0 + n1 + n2 + n3);
};

const simplexFbm = (x: number, y: number, z: number, octaves: number): number => {
  let amplitude = 1;
  let frequency = 1;
  let total = 0;
  let max = 0;

  for (let i = 0; i < octaves; i++) {
    total += simplex3(x * frequency, y * frequency, z * frequency) * amplitude;
    max += amplitude;
    amplitude *= 0.5;
    frequency *= 2;
  }

  return total / max;
};

const generateVoxelGrid = (size: number, pattern: VoxelPattern, params?: MeshJobParams): Uint16Array => {
  const voxels = new Uint16Array(size * size * size);
  const center = size / 2;
  const radius = size / 2 - 1;
  const simplexScale = params?.simplexScale ?? SIMPLEX_SCALE;
  const simplexOctaves = params?.simplexOctaves ?? SIMPLEX_OCTAVES;
  const simplexThreshold = params?.simplexThreshold ?? SIMPLEX_THRESHOLD;

  for (let z = 0; z < size; z++) {
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        const idx = x + y * size + z * size * size;
        let solid = false;

        switch (pattern) {
          case "solid":
            solid = true;
            break;
          case "checkerboard":
            solid = (x + y + z) % 2 === 0;
            break;
          case "sphere": {
            const dx = x - center;
            const dy = y - center;
            const dz = z - center;
            solid = dx * dx + dy * dy + dz * dz <= radius * radius;
            break;
          }
          case "noise":
            solid = Math.random() > 0.5;
            break;
          case "perlin": {
            const nx = x * PERLIN_SCALE;
            const ny = y * PERLIN_SCALE;
            const nz = z * PERLIN_SCALE;
            const value = perlinFbm(nx, ny, nz);
            solid = value > 0;
            break;
          }
          case "simplex": {
            const nx = x * simplexScale;
            const ny = y * simplexScale;
            const nz = z * simplexScale;
            const value = simplexFbm(nx, ny, nz, simplexOctaves);
            solid = value > simplexThreshold;
            break;
          }
        }

        voxels[idx] = solid ? 1 : 0;
      }
    }
  }

  return voxels;
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const reply = (response: MesherResponse, transfer?: Transferable[]): void => {
  if (transfer) {
    self.postMessage(response, transfer);
  } else {
    self.postMessage(response);
  }
};

const progress = (jobId: number, stage: MesherResponse & { type: "progress" } extends { stage: infer S } ? S : never): void => {
  reply({ type: "progress", jobId, stage } as MesherResponse);
};

const isCancelled = (jobId: number): boolean => currentJobId !== jobId;

// ---------------------------------------------------------------------------
// Request handlers
// ---------------------------------------------------------------------------

const handleInit = async (): Promise<void> => {
  try {
    wasmModule = await import("../wasm/wasm_greedy_mesher/wasm_greedy_mesher");
    await wasmModule.default();
    reply({ type: "init-done", version: wasmModule.get_version() });
  } catch (error) {
    reply({ type: "init-error", error: (error as Error).message });
  }
};

const handleMesh = (params: MeshJobParams): void => {
  const { jobId } = params;

  if (!wasmModule) {
    reply({ type: "mesh-error", jobId, error: "WASM module not initialized" });
    return;
  }

  currentJobId = jobId;

  try {
    // Stage 1: Generate voxel grid
    progress(jobId, "generating");
    const genStart = performance.now();
    const voxels = generateVoxelGrid(params.gridSize, params.pattern, params);
    const genTime = performance.now() - genStart;

    if (isCancelled(jobId)) return;

    // Stage 2: Mesh with WASM
    progress(jobId, "meshing");
    const meshStart = performance.now();

    if (params.debugMode) {
      meshDebug(jobId, params, voxels, genTime, meshStart);
    } else {
      meshStandard(jobId, params, voxels, genTime, meshStart);
    }
  } catch (error) {
    reply({ type: "mesh-error", jobId, error: (error as Error).message });
  }
};

const meshDebug = (
  jobId: number,
  params: MeshJobParams,
  voxels: Uint16Array,
  genTime: number,
  meshStart: number,
): void => {
  const result = wasmModule!.mesh_dense_voxels_debug(
    voxels,
    params.gridSize,
    params.gridSize,
    params.gridSize,
    params.voxelSize,
    0, 0, 0,
  );

  const meshTime = performance.now() - meshStart;

  if (result.is_empty) {
    result.free();
    reply({ type: "mesh-error", jobId, error: "empty" });
    return;
  }

  if (isCancelled(jobId)) {
    result.free();
    return;
  }

  // Stage 3: Extract data from WASM memory
  progress(jobId, "extracting");

  const positions = new Float32Array(result.positions);
  const normals = new Float32Array(result.normals);
  const indices = new Uint32Array(result.indices);
  const wirePositions = new Float32Array(result.wire_positions);
  const faceColors = new Float32Array(result.face_colors);
  const sizeColors = new Float32Array(result.size_colors);
  const dirQuadCounts = Array.from(result.dir_quad_counts);
  const dirFaceCounts = Array.from(result.dir_face_counts);

  const jobResult: MeshJobResult = {
    jobId,
    positions,
    normals,
    indices,
    wirePositions,
    faceColors,
    sizeColors,
    stats: {
      genTime,
      meshTime,
      triCount: result.triangle_count,
      vtxCount: result.vertex_count,
      quadCount: result.quad_count,
      efficiency: result.merge_efficiency,
      reduction: result.triangle_reduction,
      dirQuadCounts,
      dirFaceCounts,
    },
  };

  result.free();

  const transfer: ArrayBuffer[] = [
    positions.buffer,
    normals.buffer,
    indices.buffer,
    wirePositions.buffer,
    faceColors.buffer,
    sizeColors.buffer,
  ];

  reply({ type: "mesh-done", result: jobResult }, transfer);
};

const meshStandard = (
  jobId: number,
  params: MeshJobParams,
  voxels: Uint16Array,
  genTime: number,
  meshStart: number,
): void => {
  const result = wasmModule!.mesh_dense_voxels(
    voxels,
    params.gridSize,
    params.gridSize,
    params.gridSize,
    params.voxelSize,
    0, 0, 0,
    true,
  );

  const meshTime = performance.now() - meshStart;

  if (result.is_empty) {
    result.free();
    reply({ type: "mesh-error", jobId, error: "empty" });
    return;
  }

  if (isCancelled(jobId)) {
    result.free();
    return;
  }

  progress(jobId, "extracting");

  const positions = new Float32Array(result.positions);
  const normals = new Float32Array(result.normals);
  const indices = new Uint32Array(result.indices);

  const jobResult: MeshJobResult = {
    jobId,
    positions,
    normals,
    indices,
    stats: {
      genTime,
      meshTime,
      triCount: result.triangle_count,
      vtxCount: result.vertex_count,
    },
  };

  result.free();

  const transfer: ArrayBuffer[] = [
    positions.buffer,
    normals.buffer,
    indices.buffer,
  ];

  reply({ type: "mesh-done", result: jobResult }, transfer);
};

const handleCancel = (jobId: number): void => {
  if (currentJobId === jobId) {
    currentJobId = null;
  }
};

// ---------------------------------------------------------------------------
// Chunk Manager helpers
// ---------------------------------------------------------------------------

const cmReply = (response: ChunkManagerResponse, transfer?: Transferable[]): void => {
  if (transfer && transfer.length > 0) {
    self.postMessage(response, transfer);
  } else {
    self.postMessage(response);
  }
};

// ---------------------------------------------------------------------------
// Chunk Manager handlers
// ---------------------------------------------------------------------------

const handleCmInit = (msg: Extract<ChunkManagerRequest, { type: "cm-init" }>): void => {
  try {
    if (!wasmModule) {
      cmReply({ type: "cm-init-error", error: "WASM module not initialized. Send 'init' first." });
      return;
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const mod = wasmModule as any;

    if (msg.config) {
      if (msg.budget) {
        chunkManager = mod.WasmChunkManager.with_budget(
          msg.config.maxChunksPerFrame,
          msg.config.maxTimeMs,
          msg.config.voxelSize,
          msg.budget.maxBytes,
          msg.budget.highWatermark,
          msg.budget.lowWatermark,
          msg.budget.minChunks,
        );
      } else {
        chunkManager = mod.WasmChunkManager.with_config(
          msg.config.maxChunksPerFrame,
          msg.config.maxTimeMs,
          msg.config.voxelSize,
        );
      }
    } else {
      chunkManager = new mod.WasmChunkManager();
    }

    cmReply({ type: "cm-init-done", voxelSize: chunkManager.voxel_size() });
  } catch (error) {
    cmReply({ type: "cm-init-error", error: (error as Error).message });
  }
};

const handleCmUpdate = (msg: Extract<ChunkManagerRequest, { type: "cm-update" }>): void => {
  if (!chunkManager) {
    cmReply({ type: "cm-error", error: "Chunk manager not initialized" });
    return;
  }

  try {
    // Run update (rebuild + swap + evict)
    const wasmStats = chunkManager.update(msg.camX, msg.camY, msg.camZ);

    const stats: FrameStats = {
      chunksRebuilt: wasmStats.chunks_rebuilt,
      trianglesGenerated: wasmStats.triangles_generated,
      verticesGenerated: wasmStats.vertices_generated,
      rebuildElapsedMs: wasmStats.rebuild_elapsed_ms,
      queueRemaining: wasmStats.queue_remaining,
      timeBudgetExceeded: wasmStats.time_budget_exceeded,
      chunkLimitReached: wasmStats.chunk_limit_reached,
      meshesSwapped: wasmStats.meshes_swapped,
      meshesDisposed: wasmStats.meshes_disposed,
      versionConflicts: wasmStats.version_conflicts,
      chunksEvicted: wasmStats.chunks_evicted,
      bytesFreed: wasmStats.bytes_freed,
      totalChunks: wasmStats.total_chunks,
      chunksWithMesh: wasmStats.chunks_with_mesh,
      dirtyChunks: wasmStats.dirty_chunks,
    };
    wasmStats.free();

    // Get swapped chunk coords and extract their mesh data
    const swappedCoordsFlat: Int32Array = chunkManager.last_swapped_coords();
    const swappedMeshes: ChunkMeshTransfer[] = [];
    const transferBuffers: ArrayBuffer[] = [];

    for (let i = 0; i < swappedCoordsFlat.length; i += 3) {
      const cx = swappedCoordsFlat[i];
      const cy = swappedCoordsFlat[i + 1];
      const cz = swappedCoordsFlat[i + 2];

      const meshResult = chunkManager.get_chunk_mesh(cx, cy, cz);
      if (meshResult == null) continue;

      // Copy from WASM heap to JS-owned TypedArrays
      const positions = new Float32Array(meshResult.positions);
      const normals = new Float32Array(meshResult.normals);
      const indices = new Uint32Array(meshResult.indices);
      const uvs = new Float32Array(meshResult.uvs);
      const materialIds = new Uint16Array(meshResult.material_ids);
      const dataVersion = chunkManager.get_chunk_version(cx, cy, cz);

      meshResult.free();

      swappedMeshes.push({
        coord: { x: cx, y: cy, z: cz },
        dataVersion,
        positions,
        normals,
        indices,
        uvs,
        materialIds,
        vertexCount: positions.length / 3,
        triangleCount: indices.length / 3,
      });

      transferBuffers.push(
        positions.buffer,
        normals.buffer,
        indices.buffer,
        uvs.buffer,
        materialIds.buffer,
      );
    }

    // Get evicted chunk coords
    const evictedCoordsFlat: Int32Array = chunkManager.last_evicted_coords();
    const evictedCoords: ChunkCoord[] = [];
    for (let i = 0; i < evictedCoordsFlat.length; i += 3) {
      evictedCoords.push({
        x: evictedCoordsFlat[i],
        y: evictedCoordsFlat[i + 1],
        z: evictedCoordsFlat[i + 2],
      });
    }

    cmReply(
      { type: "cm-update-done", stats, swappedMeshes, evictedCoords },
      transferBuffers,
    );
  } catch (error) {
    cmReply({ type: "cm-error", error: (error as Error).message });
  }
};

const handleCmDebugInfo = (): void => {
  if (!chunkManager) {
    cmReply({ type: "cm-error", error: "Chunk manager not initialized" });
    return;
  }

  const wasmInfo = chunkManager.debug_info();
  const info: ChunkDebugInfo = {
    totalChunks: wasmInfo.total_chunks,
    cleanChunks: wasmInfo.clean_chunks,
    dirtyChunks: wasmInfo.dirty_chunks,
    meshingChunks: wasmInfo.meshing_chunks,
    readyToSwapChunks: wasmInfo.ready_to_swap_chunks,
    queueSize: wasmInfo.queue_size,
    totalTriangles: wasmInfo.total_triangles,
    totalVertices: wasmInfo.total_vertices,
    voxelMemoryBytes: wasmInfo.voxel_memory_bytes,
    meshMemoryBytes: wasmInfo.mesh_memory_bytes,
    budgetMaxBytes: wasmInfo.budget_max_bytes,
    budgetUsagePercent: wasmInfo.budget_usage_percent,
    budgetExceeded: wasmInfo.budget_exceeded,
  };
  wasmInfo.free();
  cmReply({ type: "cm-debug-info-result", info });
};

const handleCmGenerateAndPopulate = (
  msg: Extract<ChunkManagerRequest, { type: "cm-generate-and-populate" }>,
): void => {
  if (!wasmModule) {
    cmReply({ type: "cm-error", error: "WASM module not initialized" });
    return;
  }
  if (!chunkManager) {
    cmReply({ type: "cm-error", error: "Chunk manager not initialized" });
    return;
  }

  try {
    // 1. Generate voxel grid in JS (reuse existing pattern functions)
    const genStart = performance.now();
    const voxels = generateVoxelGrid(msg.gridSize, msg.pattern, {
      jobId: 0,
      gridSize: msg.gridSize,
      voxelSize: msg.voxelSize,
      pattern: msg.pattern,
      debugMode: false,
      debugColorMode: "none",
      debugWireframe: false,
      simplexScale: msg.simplexScale ?? SIMPLEX_SCALE,
      simplexOctaves: msg.simplexOctaves ?? SIMPLEX_OCTAVES,
      simplexThreshold: msg.simplexThreshold ?? SIMPLEX_THRESHOLD,
    });
    const genTime = performance.now() - genStart;

    // 2. Populate chunks + rebuild all (single WASM call for populate, one for rebuild)
    const meshStart = performance.now();
    chunkManager.populate_dense(voxels, msg.gridSize, msg.gridSize, msg.gridSize);
    chunkManager.rebuild_all_dirty();
    const meshTime = performance.now() - meshStart;

    // 3. Extract all swapped meshes
    const swappedCoordsFlat: Int32Array = chunkManager.last_swapped_coords();
    const swappedMeshes: ChunkMeshTransfer[] = [];
    const transferBuffers: ArrayBuffer[] = [];

    for (let i = 0; i < swappedCoordsFlat.length; i += 3) {
      const cx = swappedCoordsFlat[i];
      const cy = swappedCoordsFlat[i + 1];
      const cz = swappedCoordsFlat[i + 2];

      const meshResult = chunkManager.get_chunk_mesh(cx, cy, cz);
      if (meshResult == null) continue;

      const positions = new Float32Array(meshResult.positions);
      const normals = new Float32Array(meshResult.normals);
      const indices = new Uint32Array(meshResult.indices);
      const uvs = new Float32Array(meshResult.uvs);
      const materialIds = new Uint16Array(meshResult.material_ids);
      const dataVersion = chunkManager.get_chunk_version(cx, cy, cz);

      meshResult.free();

      swappedMeshes.push({
        coord: { x: cx, y: cy, z: cz },
        dataVersion,
        positions,
        normals,
        indices,
        uvs,
        materialIds,
        vertexCount: positions.length / 3,
        triangleCount: indices.length / 3,
      });

      transferBuffers.push(
        positions.buffer,
        normals.buffer,
        indices.buffer,
        uvs.buffer,
        materialIds.buffer,
      );
    }

    cmReply(
      {
        type: "cm-populate-done",
        chunksRebuilt: swappedMeshes.length,
        swappedMeshes,
        genTime,
        meshTime,
      },
      transferBuffers,
    );
  } catch (error) {
    cmReply({ type: "cm-error", error: (error as Error).message });
  }
};

const handleChunkManagerMessage = (msg: ChunkManagerRequest): void => {
  switch (msg.type) {
    case "cm-init":
      handleCmInit(msg);
      break;
    case "cm-update":
      handleCmUpdate(msg);
      break;
    case "cm-debug-info":
      handleCmDebugInfo();
      break;
    case "cm-set-voxel":
      if (!chunkManager) { cmReply({ type: "cm-error", error: "Not initialized" }); return; }
      chunkManager.set_voxel(msg.wx, msg.wy, msg.wz, msg.material);
      break;
    case "cm-set-voxel-at":
      if (!chunkManager) { cmReply({ type: "cm-error", error: "Not initialized" }); return; }
      chunkManager.set_voxel_at(msg.vx, msg.vy, msg.vz, msg.material);
      break;
    case "cm-set-voxels-batch":
      if (!chunkManager) { cmReply({ type: "cm-error", error: "Not initialized" }); return; }
      chunkManager.set_voxels_batch(msg.edits);
      break;
    case "cm-get-voxel":
      if (!chunkManager) { cmReply({ type: "cm-error", error: "Not initialized" }); return; }
      cmReply({ type: "cm-voxel-result", requestId: msg.requestId, material: chunkManager.get_voxel(msg.wx, msg.wy, msg.wz) });
      break;
    case "cm-set-budget":
      if (chunkManager) {
        chunkManager.set_budget(msg.budget.maxBytes, msg.budget.highWatermark, msg.budget.lowWatermark, msg.budget.minChunks);
      }
      break;
    case "cm-touch-chunk":
      chunkManager?.touch_chunk(msg.cx, msg.cy, msg.cz);
      break;
    case "cm-remove-chunk":
      chunkManager?.remove_chunk(msg.cx, msg.cy, msg.cz);
      break;
    case "cm-clear":
      chunkManager?.clear();
      break;
    case "cm-generate-and-populate":
      handleCmGenerateAndPopulate(msg);
      break;
  }
};

// ---------------------------------------------------------------------------
// Message listener
// ---------------------------------------------------------------------------

self.addEventListener("message", (e: MessageEvent<MesherRequest | ChunkManagerRequest>) => {
  const msg = e.data;

  // Route chunk manager messages by prefix
  if (typeof msg.type === "string" && msg.type.startsWith("cm-")) {
    handleChunkManagerMessage(msg as ChunkManagerRequest);
    return;
  }

  // Existing mesher messages
  switch (msg.type) {
    case "init":
      handleInit();
      break;
    case "mesh":
      handleMesh((msg as Extract<MesherRequest, { type: "mesh" }>).params);
      break;
    case "cancel":
      handleCancel((msg as Extract<MesherRequest, { type: "cancel" }>).jobId);
      break;
  }
});
