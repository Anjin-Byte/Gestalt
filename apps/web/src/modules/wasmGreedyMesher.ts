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
import { ChunkManagerClient } from "../workers/chunkManagerClient";
import type { DebugColorMode } from "../workers/mesherTypes";

/** Chunk size constant (must match Rust CS = 62). */
const CS = 62;

type MesherParams = {
  gridSize: number;
  voxelSize: number;
  pattern: "solid" | "checkerboard" | "sphere" | "noise" | "perlin" | "simplex";
  simplexScale: number;
  simplexOctaves: number;
  simplexThreshold: number;
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
  let chunkManagerClient: ChunkManagerClient | null = null;
  let chunkManagerReady = false;
  let lastVoxelSize: number | null = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;
  let updateStats: ((value: string) => void) | null = null;
  let updateDebugStats: ((value: string) => void) | null = null;

  /**
   * Multi-chunk path for grids larger than CS (62).
   * Uses ChunkManager to distribute voxels across multiple chunks.
   */
  const runMultiChunk = async (
    params: MesherParams,
    _wantsDebug: boolean,
  ): Promise<ModuleOutput[]> => {
    if (!chunkManagerClient) {
      statusText = "ChunkManager not available";
      updateStatus?.(statusText);
      return [];
    }

    try {
      // Lazy init or reinit if voxelSize changed
      if (!chunkManagerReady || lastVoxelSize !== params.voxelSize) {
        updateStatus?.("Initializing chunk manager...");
        await chunkManagerClient.initChunkManager(
          { maxChunksPerFrame: 10000, maxTimeMs: 60000, voxelSize: params.voxelSize },
          { maxBytes: 512 * 1024 * 1024, highWatermark: 0.9, lowWatermark: 0.7, minChunks: 4 },
        );
        chunkManagerReady = true;
        lastVoxelSize = params.voxelSize;
      }

      updateStatus?.("Generating & meshing chunks...");

      const result = await chunkManagerClient.generateAndPopulate({
        gridSize: params.gridSize,
        voxelSize: params.voxelSize,
        pattern: params.pattern,
        simplexScale: params.simplexScale,
        simplexOctaves: params.simplexOctaves,
        simplexThreshold: params.simplexThreshold,
      });

      const outputs: ModuleOutput[] = [];

      // Calculate totals
      let totalTriangles = 0;
      let totalVertices = 0;

      // Convert each chunk mesh to ModuleOutput
      // Note: Positions are already in world-space (Rust applies chunk origin offset)
      for (const mesh of result.swappedMeshes) {
        totalTriangles += mesh.triangleCount;
        totalVertices += mesh.vertexCount;

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

      // Add chunk boundary wireframes if enabled
      if (params.debugChunkBounds && result.swappedMeshes.length > 0) {
        for (const mesh of result.swappedMeshes) {
          const offsetX = mesh.coord.x * CS * params.voxelSize;
          const offsetY = mesh.coord.y * CS * params.voxelSize;
          const offsetZ = mesh.coord.z * CS * params.voxelSize;
          const chunkExtent = CS * params.voxelSize;

          const boundsPositions = generateBoxWireframe(
            offsetX, offsetY, offsetZ,
            offsetX + chunkExtent, offsetY + chunkExtent, offsetZ + chunkExtent,
          );
          outputs.push({
            kind: "lines",
            lines: { positions: boundsPositions, color: [0.0, 1.0, 1.0] },
            label: `Bounds (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
          });
        }
      }

      statusText = `Chunks: ${result.chunksRebuilt} | Tri: ${totalTriangles} | Vtx: ${totalVertices}`;
      updateStatus?.(statusText);

      updateStats?.(
        `Gen: ${result.genTime.toFixed(1)}ms | Mesh: ${result.meshTime.toFixed(1)}ms`
      );

      updateDebugStats?.("(Debug colors not available for multi-chunk)");

      return outputs;
    } catch (error) {
      const msg = (error as Error).message;
      if (msg === "superseded") {
        return [];
      }
      statusText = `Error: ${msg}`;
      updateStatus?.(statusText);
      updateStats?.("");
      updateDebugStats?.("");
      return [];
    }
  };

  return {
    id: "wasm-greedy-mesher",
    name: "WASM Greedy Mesher",

    init: async (ctx) => {
      ctx.logger.info("[greedy-mesher] init start");
      try {
        mesherClient = new MesherClient();
        const version = await mesherClient.init();
        // Create ChunkManagerClient sharing the same worker
        chunkManagerClient = new ChunkManagerClient(mesherClient.getWorker());
        statusText = `Loaded v${version} (worker)`;
        updateStatus?.(statusText);
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

    ui: (api) => {
      // Status displays
      api.addText({ id: "mesher-status", label: "Status", initial: statusText });
      api.addText({ id: "mesher-stats", label: "Mesh", initial: "Pending" });
      api.addText({ id: "debug-stats", label: "Debug", initial: "" });

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
        max: 256,
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
        gridSize: clamp(asInt(job.params["grid-size"], 32), 8, 256),
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
        return runMultiChunk(params, wantsDebug);
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
      chunkManagerClient?.dispose();
      chunkManagerClient = null;
      chunkManagerReady = false;
      lastVoxelSize = null;
      mesherClient?.dispose();
      mesherClient = null;
    },
  };
};
