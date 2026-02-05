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

import type { TestbedModule, ModuleOutput } from "./types";
import { MesherClient } from "../workers/mesherClient";
import type { DebugColorMode } from "../workers/mesherTypes";

type MesherParams = {
  gridSize: number;
  voxelSize: number;
  pattern: "solid" | "checkerboard" | "sphere" | "noise";
  sliceX: number;
  sliceY: number;
  sliceZ: number;
  sliceEnabled: boolean;
  debugWireframe: boolean;
  debugColorMode: DebugColorMode;
  debugChunkBounds: boolean;
};

/**
 * Generate wireframe line positions for a box from min to max.
 * Returns 12 edges × 2 endpoints × 3 floats = 72 floats.
 */
const generateBoxWireframe = (
  minX: number, minY: number, minZ: number,
  maxX: number, maxY: number, maxZ: number,
): Float32Array => {
  // 8 corners of the box
  const c = [
    [minX, minY, minZ], // 0: ---
    [maxX, minY, minZ], // 1: +--
    [maxX, maxY, minZ], // 2: ++-
    [minX, maxY, minZ], // 3: -+-
    [minX, minY, maxZ], // 4: --+
    [maxX, minY, maxZ], // 5: +-+
    [maxX, maxY, maxZ], // 6: +++
    [minX, maxY, maxZ], // 7: -++
  ];

  // 12 edges as pairs of corner indices
  const edges = [
    [0,1],[1,2],[2,3],[3,0], // bottom face
    [4,5],[5,6],[6,7],[7,4], // top face
    [0,4],[1,5],[2,6],[3,7], // verticals
  ];

  const positions = new Float32Array(72);
  let idx = 0;
  for (const [a, b] of edges) {
    positions[idx++] = c[a][0]; positions[idx++] = c[a][1]; positions[idx++] = c[a][2];
    positions[idx++] = c[b][0]; positions[idx++] = c[b][1]; positions[idx++] = c[b][2];
  }
  return positions;
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

const DIR_LABELS = ["+Y", "-Y", "+X", "-X", "+Z", "-Z"];

export const createWasmGreedyMesherModule = (): TestbedModule => {
  let mesherClient: MesherClient | null = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;
  let updateStats: ((value: string) => void) | null = null;
  let updateDebugStats: ((value: string) => void) | null = null;

  return {
    id: "wasm-greedy-mesher",
    name: "WASM Greedy Mesher",

    init: async (ctx) => {
      ctx.logger.info("[greedy-mesher] init start");
      try {
        mesherClient = new MesherClient();
        const version = await mesherClient.init();
        statusText = `Loaded v${version} (worker)`;
        updateStatus?.(statusText);
        ctx.logger.info(`[greedy-mesher] init complete: ${statusText}`);
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(`[greedy-mesher] init failed: ${(error as Error).message}`);
        mesherClient?.dispose();
        mesherClient = null;
      }
    },

    ui: (api) => {
      // Status displays
      api.addText({ id: "mesher-status", label: "Status", initial: statusText });
      api.addText({ id: "mesher-stats", label: "Mesh", initial: "Pending" });
      api.addText({ id: "debug-stats", label: "Debug", initial: "" });

      // Voxel generation controls
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
        initial: "none",
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
      updateStats = (value: string) => api.setText("mesher-stats", value);
      updateDebugStats = (value: string) => api.setText("debug-stats", value);
    },

    run: async (job) => {
      if (!mesherClient) {
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
        debugWireframe: asBool(job.params["debug-wireframe"], false),
        debugColorMode: asString(job.params["debug-color-mode"], "none") as DebugColorMode,
        debugChunkBounds: asBool(job.params["debug-chunk-bounds"], false),
      };

      const wantsDebug = params.debugWireframe || params.debugColorMode !== "none";

      try {
        const result = await mesherClient.mesh(
          {
            gridSize: params.gridSize,
            voxelSize: params.voxelSize,
            pattern: params.pattern,
            debugMode: wantsDebug,
            debugColorMode: params.debugColorMode,
            debugWireframe: params.debugWireframe,
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

          updateStats?.(
            `Gen: ${stats.genTime.toFixed(1)}ms | Mesh: ${stats.meshTime.toFixed(1)}ms | ` +
            `Efficiency: ${((stats.efficiency ?? 0) * 100).toFixed(0)}% | ${(stats.reduction ?? 0).toFixed(1)}x reduction`
          );

          if (stats.dirQuadCounts && stats.dirFaceCounts) {
            const dirParts = DIR_LABELS.map((label, i) =>
              `${label}: ${stats.dirQuadCounts![i]}q/${stats.dirFaceCounts![i]}f`
            );
            updateDebugStats?.(dirParts.join(" | "));
          }
        } else {
          outputs.push({
            kind: "mesh",
            mesh: { positions: result.positions, indices: result.indices, normals: result.normals },
            label: `Greedy Mesh (${params.pattern})`,
          });

          statusText = `Tri: ${stats.triCount} | Vtx: ${stats.vtxCount}`;
          updateStatus?.(statusText);
          updateStats?.(
            `Gen: ${stats.genTime.toFixed(1)}ms | Mesh: ${stats.meshTime.toFixed(1)}ms`
          );
          updateDebugStats?.("");
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
        updateStats?.("");
        updateDebugStats?.("");
        return [];
      }
    },

    dispose: () => {
      mesherClient?.dispose();
      mesherClient = null;
    },
  };
};
