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

const generateVoxelGrid = (size: number, pattern: VoxelPattern): Uint16Array => {
  const voxels = new Uint16Array(size * size * size);
  const center = size / 2;
  const radius = size / 2 - 1;

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
    const voxels = generateVoxelGrid(params.gridSize, params.pattern);
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
