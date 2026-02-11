/**
 * WASM Greedy Mesher testbed module.
 *
 * Demonstrates the chunk-based voxel rendering system with debug visualization:
 * - Greedy mesh with configurable voxel patterns
 * - Quad boundary wireframe overlay
 * - Face direction / merge size color modes
 * - Per-direction face count statistics
 * - Slicing with GPU clipping planes
 *
 * Meshing runs in a dedicated Web Worker so the main thread stays responsive.
 */

import type { TestbedModule, ModuleOutput } from "../types";
import { MesherClient } from "./workers/mesherClient";
import { ChunkManagerClient } from "./workers/chunkManagerClient";
import type { DebugColorMode } from "./workers/mesherTypes";
import { getDebugOverlay } from "../../ui/debugOverlay";
import {
  CS,
  DIR_LABELS,
  asBool,
  asInt,
  asNumber,
  asString,
  clamp,
  clearOverlay,
  generateBoxWireframe,
  type MesherParams,
  updateChunksOverlay,
  updateMemoryOverlay,
  updatePerformanceOverlay
} from "./helpers";
import { runMultiChunk } from "./multiChunk";

export const createWasmGreedyMesherModule = (): TestbedModule => {
  let mesherClient: MesherClient | null = null;
  let chunkManagerClient: ChunkManagerClient | null = null;
  let chunkManagerReady = false;
  let lastVoxelSize: number | null = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;

  const releaseResources = () => {
    chunkManagerClient?.dispose();
    chunkManagerClient = null;
    chunkManagerReady = false;
    lastVoxelSize = null;
    mesherClient?.dispose();
    mesherClient = null;
  };

  const ensureClients = async () => {
    if (mesherClient && chunkManagerClient) {
      return;
    }
    mesherClient = new MesherClient();
    const version = await mesherClient.init();
    chunkManagerClient = new ChunkManagerClient(mesherClient.getWorker());
    statusText = `Loaded v${version} (worker)`;
    updateStatus?.(statusText);
  };

  return {
    id: "wasm-greedy-mesher",
    name: "WASM Greedy Mesher",

    init: async (ctx) => {
      ctx.logger.info("[greedy-mesher] init start");
      try {
        await ensureClients();
        ctx.logger.info(`[greedy-mesher] init complete: ${statusText}`);
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(`[greedy-mesher] init failed: ${(error as Error).message}`);
        chunkManagerClient?.dispose();
        chunkManagerClient = null;
        mesherClient?.dispose();
        mesherClient = null;
      }
    },
    activate: async () => {
      await ensureClients();
    },

    ui: (api) => {
      // Module status (loading state)
      api.addText({ id: "mesher-status", label: "Status", initial: statusText });

      // Voxel generation controls
      api.addSelect({
        id: "pattern",
        label: "Voxel Pattern",
        options: ["solid", "checkerboard", "sphere", "noise", "perlin", "simplex"],
        initial: "simplex",
      });

      api.addNumber({
        id: "grid-size",
        label: "Grid Size",
        min: 8,
        max: 1024,
        step: 1,
        initial: 62,
      });

      api.addNumber({
        id: "voxel-size",
        label: "Voxel Size",
        min: 0.01,
        max: 0.5,
        step: 0.01,
        initial: 0.1,
      });

      api.addNumber({
        id: "simplex-scale",
        label: "Simplex Scale",
        min: 0.01,
        max: 0.5,
        step: 0.01,
        initial: 0.02,
      });

      api.addNumber({
        id: "simplex-octaves",
        label: "Simplex Octaves",
        min: 1,
        max: 6,
        step: 1,
        initial: 3,
      });

      api.addNumber({
        id: "simplex-threshold",
        label: "Simplex Threshold",
        min: -1,
        max: 1,
        step: 0.05,
        initial: 0,
      });

      // Debug visualization controls
      api.addCheckbox({
        id: "debug-wireframe",
        label: "Quad Wireframe",
        initial: false,
      });

      api.addSelect({
        id: "debug-color-mode",
        label: "Color Mode",
        options: ["none", "face-direction", "quad-size"],
        initial: "quad-size",
      });

      api.addCheckbox({
        id: "debug-chunk-bounds",
        label: "Chunk Bounds",
        initial: false,
      });

      // Slicing controls
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

      updateStatus = (value: string) => api.setText("mesher-status", value);
    },

    run: async (job) => {
      if (!mesherClient) {
        statusText = "Not loaded";
        updateStatus?.(statusText);
        return [];
      }

      const params: MesherParams = {
        gridSize: clamp(asInt(job.params["grid-size"], 32), 8, 1024),
        voxelSize: clamp(asNumber(job.params["voxel-size"], 0.1), 0.01, 0.5),
        pattern: asString(job.params.pattern, "sphere") as MesherParams["pattern"],
        simplexScale: clamp(asNumber(job.params["simplex-scale"], 0.08), 0.01, 0.5),
        simplexOctaves: clamp(asInt(job.params["simplex-octaves"], 3), 1, 6),
        simplexThreshold: clamp(asNumber(job.params["simplex-threshold"], 0), -1, 1),
        sliceX: asNumber(job.params["slice-x"], 0),
        sliceY: asNumber(job.params["slice-y"], 0),
        sliceZ: asNumber(job.params["slice-z"], 0),
        sliceEnabled: asBool(job.params["slice-enabled"], false),
        debugWireframe: asBool(job.params["debug-wireframe"], false),
        debugColorMode: asString(job.params["debug-color-mode"], "none") as DebugColorMode,
        debugChunkBounds: asBool(job.params["debug-chunk-bounds"], false),
      };

      const wantsDebug = params.debugWireframe || params.debugColorMode !== "none";

      // Route to multi-chunk path for large grids
      if (params.gridSize > CS) {
        const multiChunkState = {
          chunkManagerClient,
          chunkManagerReady,
          lastVoxelSize,
          statusText,
          updateStatus
        };
        const outputs = await runMultiChunk(
          params,
          multiChunkState,
          {
            updatePerformanceOverlay,
            updateChunksOverlay,
            updateMemoryOverlay,
            clearOverlay
          }
        );
        chunkManagerReady = multiChunkState.chunkManagerReady;
        lastVoxelSize = multiChunkState.lastVoxelSize;
        statusText = multiChunkState.statusText;
        return outputs;
      }

      try {
        const result = await mesherClient.mesh(
          {
            gridSize: params.gridSize,
            voxelSize: params.voxelSize,
            pattern: params.pattern,
            debugMode: wantsDebug,
            debugColorMode: params.debugColorMode,
            debugWireframe: params.debugWireframe,
            simplexScale: params.simplexScale,
            simplexOctaves: params.simplexOctaves,
            simplexThreshold: params.simplexThreshold,
          },
          (stage) => {
            if (stage === "generating") updateStatus?.("Generating voxels...");
            else if (stage === "meshing") updateStatus?.("Meshing...");
            else if (stage === "extracting") updateStatus?.("Extracting data...");
          },
        );

        const outputs: ModuleOutput[] = [];
        const { stats } = result;

        if (wantsDebug) {
          // Determine vertex colors based on mode
          let colors: Float32Array | undefined;
          if (params.debugColorMode === "face-direction") {
            colors = result.faceColors;
          } else if (params.debugColorMode === "quad-size") {
            colors = result.sizeColors;
          }

          outputs.push({
            kind: "mesh",
            mesh: { positions: result.positions, indices: result.indices, normals: result.normals, colors },
            label: `Greedy Mesh (${params.pattern})`,
          });

          if (params.debugWireframe && result.wirePositions && result.wirePositions.length > 0) {
            outputs.push({
              kind: "lines",
              lines: { positions: result.wirePositions, color: [1.0, 1.0, 0.0] },
              label: "Quad Boundaries",
            });
          }

          statusText = `Tri: ${stats.triCount} | Vtx: ${stats.vtxCount} | Quads: ${stats.quadCount}`;
          updateStatus?.(statusText);

          // Update overlay with single-chunk stats
          updatePerformanceOverlay(
            stats.genTime,
            stats.meshTime,
            `${((stats.efficiency ?? 0) * 100).toFixed(0)}% eff, ${(stats.reduction ?? 0).toFixed(1)}x red`,
          );
          updateChunksOverlay(1, stats.triCount, stats.vtxCount, stats.quadCount);

          // Show per-direction stats in custom section
          if (stats.dirQuadCounts && stats.dirFaceCounts) {
            const overlay = getDebugOverlay();
            overlay?.update("custom", DIR_LABELS.map((label, i) => ({
              label,
              value: `${stats.dirQuadCounts![i]}q / ${stats.dirFaceCounts![i]}f`,
            })));
          }
        } else {
          outputs.push({
            kind: "mesh",
            mesh: { positions: result.positions, indices: result.indices, normals: result.normals },
            label: `Greedy Mesh (${params.pattern})`,
          });

          statusText = `Tri: ${stats.triCount} | Vtx: ${stats.vtxCount}`;
          updateStatus?.(statusText);

          updatePerformanceOverlay(stats.genTime, stats.meshTime);
          updateChunksOverlay(1, stats.triCount, stats.vtxCount);
        }

        // Chunk boundary wireframe (trivial — stays on main thread)
        if (params.debugChunkBounds) {
          const extent = params.gridSize * params.voxelSize;
          const boundsPositions = generateBoxWireframe(0, 0, 0, extent, extent, extent);
          outputs.push({
            kind: "lines",
            lines: { positions: boundsPositions, color: [0.0, 1.0, 1.0] },
            label: "Chunk Bounds",
          });
        }

        return outputs;
      } catch (error) {
        const msg = (error as Error).message;
        if (msg === "cancelled") {
          // Job was superseded by a newer request — not an error
          return [];
        }
        if (msg === "empty") {
          statusText = "Empty mesh";
          updateStatus?.(statusText);
          return [];
        }
        statusText = `Error: ${msg}`;
        updateStatus?.(statusText);
        clearOverlay();
        return [];
      }
    },

    deactivate: () => {
      releaseResources();
    },
    dispose: () => {
      releaseResources();
    },
  };
};
