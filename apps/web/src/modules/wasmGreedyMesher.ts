/**
 * WASM Greedy Mesher testbed module.
 *
 * Demonstrates the chunk-based voxel rendering system:
 * - ChunkMeshPool for mesh management
 * - Double buffering for flicker-free updates
 * - SlicingManager for clipping planes
 */

import type { TestbedModule } from "./types";
import { ChunkRenderManager } from "../voxel";
import type { ChunkCoord } from "../voxel";

type MesherParams = {
  gridSize: number;
  voxelSize: number;
  pattern: "solid" | "checkerboard" | "sphere" | "noise";
  sliceX: number;
  sliceY: number;
  sliceZ: number;
  sliceEnabled: boolean;
  showStats: boolean;
};

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

const asNumber = (value: unknown, fallback: number): number => {
  const num = Number(value);
  return Number.isFinite(num) ? num : fallback;
};

const asInt = (value: unknown, fallback: number): number =>
  Math.floor(asNumber(value, fallback));

const asBool = (value: unknown, fallback: boolean): boolean =>
  typeof value === "boolean" ? value : fallback;

const asString = (value: unknown, fallback: string): string =>
  typeof value === "string" ? value : fallback;

/**
 * Generate voxel grid based on pattern.
 */
const generateVoxelGrid = (
  size: number,
  pattern: MesherParams["pattern"]
): Uint16Array => {
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

export const createWasmGreedyMesherModule = (): TestbedModule => {
  let wasmModule: typeof import("../wasm/wasm_greedy_mesher/wasm_greedy_mesher") | null = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;
  let updateStats: ((value: string) => void) | null = null;
  let renderManager: ChunkRenderManager | null = null;

  return {
    id: "wasm-greedy-mesher",
    name: "WASM Greedy Mesher",

    init: async (ctx) => {
      ctx.logger.info("[greedy-mesher] init start");
      try {
        wasmModule = await import("../wasm/wasm_greedy_mesher/wasm_greedy_mesher");
        await wasmModule.default();
        statusText = `Loaded v${wasmModule.get_version()}`;
        updateStatus?.(statusText);
        ctx.logger.info(`[greedy-mesher] init complete: ${statusText}`);
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(`[greedy-mesher] init failed: ${(error as Error).message}`);
      }
    },

    ui: (api) => {
      api.addText({ id: "mesher-status", label: "Status", initial: statusText });
      api.addText({ id: "mesher-stats", label: "Stats", initial: "Pending" });

      api.addSelect({
        id: "pattern",
        label: "Voxel Pattern",
        options: ["solid", "checkerboard", "sphere", "noise"],
        initial: "sphere",
      });

      api.addNumber({
        id: "grid-size",
        label: "Grid Size",
        min: 8,
        max: 62,
        step: 1,
        initial: 32,
      });

      api.addNumber({
        id: "voxel-size",
        label: "Voxel Size",
        min: 0.01,
        max: 0.5,
        step: 0.01,
        initial: 0.1,
      });

      api.addCheckbox({
        id: "slice-enabled",
        label: "Enable Slicing",
        initial: false,
      });

      api.addNumber({
        id: "slice-x",
        label: "Slice X",
        min: -5,
        max: 5,
        step: 0.1,
        initial: 0,
      });

      api.addNumber({
        id: "slice-y",
        label: "Slice Y",
        min: -5,
        max: 5,
        step: 0.1,
        initial: 0,
      });

      api.addNumber({
        id: "slice-z",
        label: "Slice Z",
        min: -5,
        max: 5,
        step: 0.1,
        initial: 0,
      });

      api.addCheckbox({
        id: "show-stats",
        label: "Show Stats",
        initial: true,
      });

      updateStatus = (value: string) => api.setText("mesher-status", value);
      updateStats = (value: string) => api.setText("mesher-stats", value);
    },

    run: async (job) => {
      if (!wasmModule) {
        statusText = "Not loaded";
        updateStatus?.(statusText);
        return [];
      }

      const params: MesherParams = {
        gridSize: clamp(asInt(job.params["grid-size"], 32), 8, 62),
        voxelSize: clamp(asNumber(job.params["voxel-size"], 0.1), 0.01, 0.5),
        pattern: asString(job.params.pattern, "sphere") as MesherParams["pattern"],
        sliceX: asNumber(job.params["slice-x"], 0),
        sliceY: asNumber(job.params["slice-y"], 0),
        sliceZ: asNumber(job.params["slice-z"], 0),
        sliceEnabled: asBool(job.params["slice-enabled"], false),
        showStats: asBool(job.params["show-stats"], true),
      };

      const startTime = performance.now();

      // Generate voxel data
      const voxels = generateVoxelGrid(params.gridSize, params.pattern);
      const genTime = performance.now() - startTime;

      // Mesh with WASM
      const meshStart = performance.now();
      const result = wasmModule.mesh_dense_voxels(
        voxels,
        params.gridSize,
        params.gridSize,
        params.gridSize,
        params.voxelSize,
        0, 0, 0, // origin
        true // generate UVs
      );
      const meshTime = performance.now() - meshStart;

      if (result.is_empty) {
        statusText = "Empty mesh";
        updateStatus?.(statusText);
        result.free();
        return [];
      }

      // Create render manager if needed
      if (!renderManager) {
        renderManager = new ChunkRenderManager({
          voxelSize: params.voxelSize,
          chunkSize: 62,
        });
      }

      // Update chunk mesh
      const coord: ChunkCoord = { x: 0, y: 0, z: 0 };
      renderManager.setChunkMeshFromWasm(coord, result, job.frameId);
      renderManager.swapPendingMeshes();

      // Configure slicing
      renderManager.setSlicingEnabled(params.sliceEnabled);
      if (params.sliceEnabled) {
        renderManager.setSliceEnabled("x", true);
        renderManager.setSliceEnabled("y", true);
        renderManager.setSliceEnabled("z", true);
        renderManager.setSlicePosition("x", params.sliceX);
        renderManager.setSlicePosition("y", params.sliceY);
        renderManager.setSlicePosition("z", params.sliceZ);
      }

      // Copy data before freeing result
      const triangleCount = result.triangle_count;
      const vertexCount = result.vertex_count;
      const positions = new Float32Array(result.positions);
      const indices = new Uint32Array(result.indices);
      const normals = new Float32Array(result.normals);

      // Get stats
      const stats = renderManager.getStats();

      // Update status
      statusText = `Triangles: ${triangleCount} | Vertices: ${vertexCount}`;
      updateStatus?.(statusText);

      if (params.showStats) {
        updateStats?.(
          `Gen: ${genTime.toFixed(1)}ms | Mesh: ${meshTime.toFixed(1)}ms | ` +
          `Chunks: ${stats.totalChunks} | Pending: ${stats.pendingSwaps}`
        );
      }

      // Free WASM result
      result.free();

      // Return the mesh output for the viewer
      return [{
        kind: "mesh",
        mesh: {
          positions,
          indices,
          normals,
        },
        label: `Greedy Mesh (${params.pattern})`,
      }];
    },

    dispose: () => {
      if (renderManager) {
        renderManager.dispose();
        renderManager = null;
      }
    },
  };
};
