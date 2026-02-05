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

declare const self: DedicatedWorkerGlobalScope;

type WasmModule = typeof import("../wasm/wasm_greedy_mesher/wasm_greedy_mesher");

let wasmModule: WasmModule | null = null;
let currentJobId: number | null = null;

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
// Message listener
// ---------------------------------------------------------------------------

self.addEventListener("message", (e: MessageEvent<MesherRequest>) => {
  switch (e.data.type) {
    case "init":
      handleInit();
      break;
    case "mesh":
      handleMesh(e.data.params);
      break;
    case "cancel":
      handleCancel(e.data.jobId);
      break;
  }
});
