import type { ModuleOutput, ModuleContext, TestbedModule } from "../types";
import { MesherClient } from "../wasmGreedyMesher/workers/mesherClient";
import { ChunkManagerClient } from "../wasmGreedyMesher/workers/chunkManagerClient";
import type { ChunkMeshTransfer } from "../wasmGreedyMesher/workers/chunkManagerTypes";
import { parseObjFallback, buildMaterialTable } from "../wasmObjLoader/helpers";
import { packMaterialTable } from "../wasmGreedyMesher/voxelizeToChunks";
import { defaultSampleModels, type SampleModel } from "../wasmVoxelizer/helpers";
import { getDebugOverlay } from "../../ui/debugOverlay";
import { filterCompactVoxelsByLattice, countCompactVoxels } from "./compactOps";
import { clamp, asBool, asInt, asNumber, asString, computeGridOrigin, normalizeMeshInPlace } from "./helpers";
import { getTopology } from "./topologies";
import type { LatticeRunParams } from "./types";

type WasmVoxelizerModule = {
  default?: () => Promise<unknown>;
  WasmVoxelizer?: {
    new?: () => Promise<WasmVoxelizerInstance>;
  };
};

type WasmVoxelizerInstance = {
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
};

const computeGOrigin = (
  origin: [number, number, number],
  voxelSize: number
): [number, number, number] => [
  Math.floor(origin[0] / voxelSize),
  Math.floor(origin[1] / voxelSize),
  Math.floor(origin[2] / voxelSize),
];

const hslToRgb = (h: number, s: number, l: number): [number, number, number] => {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h * 6) % 2) - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;
  const sector = Math.floor(h * 6) % 6;
  if (sector === 0) { r = c; g = x; }
  else if (sector === 1) { r = x; g = c; }
  else if (sector === 2) { g = c; b = x; }
  else if (sector === 3) { g = x; b = c; }
  else if (sector === 4) { r = x; b = c; }
  else { r = c; b = x; }
  return [r + m, g + m, b + m];
};

const materialColor = (materialId: number): [number, number, number] => {
  if (materialId <= 1) return [0.7, 0.7, 0.7];
  const hue = ((materialId * 0.618033988749895) % 1.0);
  return hslToRgb(hue, 0.7, 0.55);
};

const chunkColor = (cx: number, cy: number, cz: number): [number, number, number] => {
  const hash = ((cx * 73856093) ^ (cy * 19349663) ^ (cz * 83492791)) >>> 0;
  const hue = (hash % 360) / 360;
  return hslToRgb(hue, 0.65, 0.5);
};

const FACE_DIR_COLORS: [number, number, number][] = [
  [0.35, 0.85, 0.35],
  [0.85, 0.35, 0.35],
  [0.35, 0.35, 0.85],
  [0.85, 0.85, 0.35],
  [0.85, 0.35, 0.85],
  [0.35, 0.85, 0.85],
];

const normalToFaceDir = (nx: number, ny: number, nz: number): number => {
  const ax = Math.abs(nx), ay = Math.abs(ny), az = Math.abs(nz);
  if (ay >= ax && ay >= az) return ny >= 0 ? 0 : 1;
  if (ax >= ay && ax >= az) return nx >= 0 ? 2 : 3;
  return nz >= 0 ? 4 : 5;
};

const quadSizeColor = (width: number, height: number): [number, number, number] => {
  const area = width * height;
  const maxLog = Math.log(62 * 62);
  const t = Math.max(0, Math.min(1, Math.log(Math.max(1, area)) / maxLog));
  return [1 - t, t, 0.1];
};

const generateColors = (
  mesh: ChunkMeshTransfer,
  mode: "material" | "chunk" | "face-direction" | "quad-size",
): Float32Array => {
  const vertCount = mesh.positions.length / 3;
  const colors = new Float32Array(vertCount * 3);

  if (mode === "material") {
    for (let i = 0; i < vertCount; i++) {
      const [r, g, b] = materialColor(mesh.materialIds[i] ?? 0);
      colors[i * 3] = r;
      colors[i * 3 + 1] = g;
      colors[i * 3 + 2] = b;
    }
  } else if (mode === "chunk") {
    const [r, g, b] = chunkColor(mesh.coord.x, mesh.coord.y, mesh.coord.z);
    for (let i = 0; i < vertCount; i++) {
      colors[i * 3] = r;
      colors[i * 3 + 1] = g;
      colors[i * 3 + 2] = b;
    }
  } else if (mode === "face-direction") {
    for (let i = 0; i < vertCount; i++) {
      const dir = normalToFaceDir(
        mesh.normals[i * 3],
        mesh.normals[i * 3 + 1],
        mesh.normals[i * 3 + 2]
      );
      const [r, g, b] = FACE_DIR_COLORS[dir];
      colors[i * 3] = r;
      colors[i * 3 + 1] = g;
      colors[i * 3 + 2] = b;
    }
  } else if (mode === "quad-size") {
    for (let q = 0; q < vertCount; q += 4) {
      let maxU = 0, maxV = 0;
      for (let v = 0; v < 4 && q + v < vertCount; v++) {
        const u = Math.abs(mesh.uvs[(q + v) * 2] ?? 0);
        const vv = Math.abs(mesh.uvs[(q + v) * 2 + 1] ?? 0);
        if (u > maxU) maxU = u;
        if (vv > maxV) maxV = vv;
      }
      const [r, g, b] = quadSizeColor(Math.max(1, Math.round(maxU)), Math.max(1, Math.round(maxV)));
      for (let v = 0; v < 4 && q + v < vertCount; v++) {
        colors[(q + v) * 3] = r;
        colors[(q + v) * 3 + 1] = g;
        colors[(q + v) * 3 + 2] = b;
      }
    }
  }

  return colors;
};

const generateWireframe = (mesh: ChunkMeshTransfer): Float32Array => {
  const vertCount = mesh.positions.length / 3;
  const quadCount = Math.floor(vertCount / 4);
  const lines = new Float32Array(quadCount * 24);
  let idx = 0;

  for (let q = 0; q < quadCount; q++) {
    const base = q * 4;
    for (let e = 0; e < 4; e++) {
      const a = base + e;
      const b = base + ((e + 1) % 4);
      lines[idx++] = mesh.positions[a * 3];
      lines[idx++] = mesh.positions[a * 3 + 1];
      lines[idx++] = mesh.positions[a * 3 + 2];
      lines[idx++] = mesh.positions[b * 3];
      lines[idx++] = mesh.positions[b * 3 + 1];
      lines[idx++] = mesh.positions[b * 3 + 2];
    }
  }

  return lines;
};

const compactToVoxelPositions = (
  voxels: Int32Array,
  origin: [number, number, number],
  voxelSize: number
): Float32Array => {
  const positions = new Float32Array((voxels.length / 4) * 3);
  let out = 0;
  for (let i = 0; i < voxels.length; i += 4) {
    positions[out++] = origin[0] + (voxels[i] + 0.5) * voxelSize;
    positions[out++] = origin[1] + (voxels[i + 1] + 0.5) * voxelSize;
    positions[out++] = origin[2] + (voxels[i + 2] + 0.5) * voxelSize;
  }
  return positions;
};

const meshOutputsFromChunks = (
  outputs: ModuleOutput[],
  swappedMeshes: Awaited<ReturnType<ChunkManagerClient["rebuildAllDirty"]>>["swappedMeshes"],
  debugChunkBounds: boolean,
  debugWireframe: boolean,
  colorMode: "none" | "material" | "chunk" | "face-direction" | "quad-size",
  voxelSize: number
) => {
  for (const mesh of swappedMeshes) {
    outputs.push({
      kind: "mesh",
      mesh: {
        positions: mesh.positions,
        indices: mesh.indices,
        normals: mesh.normals,
        colors: colorMode === "none" ? undefined : generateColors(mesh, colorMode),
      },
      label: `Lattice Chunk (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
    });

    if (debugWireframe) {
      outputs.push({
        kind: "lines",
        lines: {
          positions: generateWireframe(mesh),
          color: [0.05, 0.05, 0.05],
        },
        label: `Quad Wireframe (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
      });
    }
  }

  if (!debugChunkBounds) {
    return;
  }

  const CS = 62;
  for (const mesh of swappedMeshes) {
    const minX = mesh.coord.x * CS * voxelSize;
    const minY = mesh.coord.y * CS * voxelSize;
    const minZ = mesh.coord.z * CS * voxelSize;
    const maxX = minX + CS * voxelSize;
    const maxY = minY + CS * voxelSize;
    const maxZ = minZ + CS * voxelSize;
    const corners = [
      [minX, minY, minZ], [maxX, minY, minZ], [maxX, maxY, minZ], [minX, maxY, minZ],
      [minX, minY, maxZ], [maxX, minY, maxZ], [maxX, maxY, maxZ], [minX, maxY, maxZ],
    ];
    const edges = [
      [0, 1], [1, 2], [2, 3], [3, 0],
      [4, 5], [5, 6], [6, 7], [7, 4],
      [0, 4], [1, 5], [2, 6], [3, 7],
    ];
    const positions: number[] = [];
    for (const [a, b] of edges) {
      positions.push(...corners[a], ...corners[b]);
    }
    outputs.push({
      kind: "lines",
      lines: {
        positions: new Float32Array(positions),
        color: [0.0, 1.0, 1.0],
      },
      label: `Bounds (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`,
    });
  }
};

export const createLatticeChunkPipelineModule = (): TestbedModule => {
  let voxelizer: WasmVoxelizerInstance | null = null;
  let mesherClient: MesherClient | null = null;
  let chunkManagerClient: ChunkManagerClient | null = null;

  let statusText = "Not loaded";
  let fileName = "No file";
  let objText = "";
  let hasUploadedFile = false;
  let updateStatus: ((value: string) => void) | null = null;
  let updateFileName: ((value: string) => void) | null = null;
  let logger: ((msg: string) => void) | null = null;

  let sampleModels: SampleModel[] = defaultSampleModels;
  let sampleId = sampleModels[0]?.id ?? "";
  let sampleText = "";
  const sampleCache = new Map<string, string>();
  let ctxRef: ModuleContext | null = null;

  const loadSampleModel = async (id: string): Promise<string> => {
    const cached = sampleCache.get(id);
    if (cached) return cached;
    const entry = sampleModels.find((m) => m.id === id) ?? sampleModels[0];
    if (!entry || !ctxRef) return "";
    const base = new URL(ctxRef.baseUrl || "/", window.location.href);
    const url = new URL(entry.file, base).toString();
    const response = await fetch(url);
    if (!response.ok) return "";
    const text = await response.text();
    sampleCache.set(entry.id, text);
    return text;
  };

  const releaseResources = () => {
    chunkManagerClient?.dispose();
    chunkManagerClient = null;
    mesherClient?.dispose();
    mesherClient = null;
    voxelizer = null;
  };

  const ensureVoxelizer = async () => {
    if (voxelizer) return;
    const module: WasmVoxelizerModule = await import(
      "../../wasm/wasm_voxelizer/wasm_voxelizer.js"
    );
    if (module.default) await module.default();
    const instance = await module.WasmVoxelizer?.new?.();
    if (!instance) throw new Error("WasmVoxelizer exports missing");
    voxelizer = instance;
  };

  const ensureMesher = async () => {
    if (mesherClient && chunkManagerClient) return;
    mesherClient = new MesherClient();
    await mesherClient.init();
    chunkManagerClient = new ChunkManagerClient(mesherClient.getWorker());
  };

  const voxelizeHostToCompact = async (
    positions: Float32Array,
    indices: Uint32Array,
    materialTable: Uint32Array,
    origin: [number, number, number],
    voxelSize: number,
    gridDim: number,
    epsilon: number,
  ) => {
    if (!voxelizer) {
      throw new Error("Voxelizer not available");
    }
    const gOrigin = computeGOrigin(origin, voxelSize);
    const result = await voxelizer.voxelize_compact_voxels(
      positions,
      indices,
      materialTable,
      new Float32Array(origin),
      voxelSize,
      new Uint32Array([gridDim, gridDim, gridDim]),
      epsilon,
      gOrigin[0],
      gOrigin[1],
      gOrigin[2],
    );
    return result.voxels;
  };

  return {
    id: "lattice-chunk-pipeline",
    name: "Lattice Chunk Pipeline",

    init: async (ctx: ModuleContext) => {
      logger = ctx.logger.info;
      ctxRef = ctx;
      try {
        await Promise.all([ensureVoxelizer(), ensureMesher()]);
        sampleText = await loadSampleModel(sampleId);
        if (sampleText) {
          fileName = sampleModels.find((m) => m.id === sampleId)?.label ?? sampleId;
          updateFileName?.(fileName);
        }
        statusText = "Loaded";
        updateStatus?.(statusText);
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error);
        statusText = "Missing (run pnpm build:wasm:legacy)";
        updateStatus?.(statusText);
        ctx.logger.warn(`[lattice-chunk-pipeline] init failed: ${msg}`);
        releaseResources();
      }
    },

    activate: async () => {
      await Promise.all([ensureVoxelizer(), ensureMesher()]);
    },

    ui: (api) => {
      api.addText({ id: "status", label: "Status", initial: statusText });
      api.addText({ id: "obj-file", label: "OBJ File", initial: fileName });
      api.addSelect({
        id: "sample-model",
        label: "Sample Model",
        options: sampleModels.map((m) => m.label),
        initial: sampleModels.find((m) => m.id === sampleId)?.label ?? sampleModels[0]?.label ?? "",
      });
      api.addFile({
        id: "obj-input",
        label: "Pick OBJ",
        accept: ".obj",
        onFile: async (file) => {
          if (!file) {
            fileName = "No file";
            objText = "";
            hasUploadedFile = false;
            updateFileName?.(fileName);
            updateStatus?.("No file selected");
            return;
          }
          fileName = file.name;
          hasUploadedFile = true;
          updateFileName?.(fileName);
          updateStatus?.("Loading...");
          try {
            objText = await file.text();
            updateStatus?.(`Ready (${objText.length} chars)`);
          } catch {
            updateStatus?.("File read failed");
            objText = "";
          }
        },
      });

      api.addNumber({ id: "grid-dim", label: "Grid Dim", min: 8, max: 4096, step: 8, initial: 256 });
      api.addNumber({ id: "voxel-size", label: "Voxel Size", min: 0.001, max: 0.5, step: 0.001, initial: 0.1 });
      api.addNumber({ id: "epsilon", label: "Epsilon", min: 0, max: 0.01, step: 0.0001, initial: 0.0001 });
      api.addCheckbox({ id: "fit-bounds", label: "Fit Grid To Mesh", initial: true });
      api.addSelect({ id: "lattice-type", label: "Lattice Type", options: ["cubic", "bcc", "kelvin"], initial: "cubic" });
      api.addNumber({ id: "cell-size", label: "Cell Size", min: 0.01, max: 2.0, step: 0.01, initial: 0.12 });
      api.addNumber({ id: "strut-radius", label: "Strut Radius", min: 0.001, max: 0.5, step: 0.001, initial: 0.02 });
      api.addSelect({ id: "origin-mode", label: "Origin Mode", options: ["centered", "grid-origin"], initial: "centered" });
      api.addCheckbox({ id: "show-host-voxels", label: "Show Host Voxels", initial: false });
      api.addCheckbox({ id: "show-result-voxels", label: "Show Result Voxels", initial: false });
      api.addSelect({ id: "color-mode", label: "Color Mode", options: ["none", "material", "chunk", "face-direction", "quad-size"], initial: "material" });
      api.addCheckbox({ id: "debug-wireframe", label: "Quad Wireframe", initial: false });
      api.addCheckbox({ id: "debug-chunk-bounds", label: "Chunk Bounds", initial: false });

      updateStatus = (v: string) => api.setText("status", v);
      updateFileName = (v: string) => api.setText("obj-file", v);
    },

    run: async (job) => {
      if (!voxelizer || !chunkManagerClient) {
        updateStatus?.("Not loaded");
        return [];
      }

      const selectedLabel = String(job.params["sample-model"] ?? sampleModels[0]?.label ?? "");
      const selectedModel = sampleModels.find((m) => m.label === selectedLabel) ?? sampleModels[0];
      const selectedId = selectedModel?.id ?? sampleId;
      if (!hasUploadedFile && selectedId !== sampleId) {
        sampleId = selectedId;
        sampleText = "";
      }
      if (!hasUploadedFile && !sampleText) {
        updateStatus?.("Loading sample model...");
        sampleText = await loadSampleModel(sampleId);
        if (sampleText) {
          fileName = selectedModel?.label ?? sampleId;
          updateFileName?.(fileName);
        }
      }

      const sourceText = hasUploadedFile ? objText : sampleText;
      if (!sourceText) {
        updateStatus?.("No OBJ loaded");
        return [];
      }

      const params: LatticeRunParams = {
        gridDim: clamp(asInt(job.params["grid-dim"], 256), 8, 4096),
        voxelSize: clamp(asNumber(job.params["voxel-size"], 0.1), 0.001, 0.5),
        epsilon: clamp(asNumber(job.params.epsilon, 0.0001), 0, 0.01),
        fitBounds: asBool(job.params["fit-bounds"], true),
        debugChunkBounds: asBool(job.params["debug-chunk-bounds"], false),
        debugWireframe: asBool(job.params["debug-wireframe"], false),
        colorMode: asString(job.params["color-mode"], "material") as LatticeRunParams["colorMode"],
        showHostVoxels: asBool(job.params["show-host-voxels"], false),
        showResultVoxels: asBool(job.params["show-result-voxels"], false),
        lattice: {
          latticeType: asString(job.params["lattice-type"], "cubic") as LatticeRunParams["lattice"]["latticeType"],
          cellSize: clamp(asNumber(job.params["cell-size"], 0.12), 0.01, 2.0),
          strutRadius: clamp(asNumber(job.params["strut-radius"], 0.02), 0.001, 0.5),
          originMode: asString(job.params["origin-mode"], "centered") as LatticeRunParams["lattice"]["originMode"],
          latticeOrigin: [0, 0, 0],
        },
      };

      ctxRef?.clearChunkOutputs?.();
      updateStatus?.("Parsing OBJ...");
      const parsed = parseObjFallback(sourceText);
      if (parsed.positions.length === 0 || parsed.indices.length === 0) {
        updateStatus?.("No faces found in OBJ");
        return [];
      }

      normalizeMeshInPlace(parsed.positions);
      const grid = computeGridOrigin(parsed.positions, params.gridDim, params.voxelSize, params.fitBounds);
      params.voxelSize = grid.voxelSize;
      if (params.lattice.originMode === "centered") {
        params.lattice.latticeOrigin = [
          grid.origin[0] + (params.gridDim * params.voxelSize) * 0.5,
          grid.origin[1] + (params.gridDim * params.voxelSize) * 0.5,
          grid.origin[2] + (params.gridDim * params.voxelSize) * 0.5,
        ];
      } else {
        params.lattice.latticeOrigin = grid.origin;
      }

      const materialTable = packMaterialTable(buildMaterialTable(parsed.triangleMaterials, parsed.materialGroupNames));
      const topology = getTopology(params.lattice.latticeType);

      updateStatus?.("Initializing chunk manager...");
      await chunkManagerClient.initChunkManager(
        { maxChunksPerFrame: 10000, maxTimeMs: 60000, voxelSize: params.voxelSize },
        { maxBytes: 512 * 1024 * 1024, highWatermark: 0.9, lowWatermark: 0.7, minChunks: 4 },
      );

      try {
        updateStatus?.("Voxelizing host...");
        const hostCompact = await voxelizeHostToCompact(
          parsed.positions,
          parsed.indices,
          materialTable,
          grid.origin,
          params.voxelSize,
          params.gridDim,
          params.epsilon,
        );

        updateStatus?.("Applying lattice mask...");
        const resultCompact = filterCompactVoxelsByLattice(
          hostCompact,
          grid.origin,
          params.voxelSize,
          topology,
          params.lattice,
        );

        updateStatus?.("Meshing chunks...");
        await chunkManagerClient.ingestCompactVoxels(resultCompact);
        const rebuild = await chunkManagerClient.rebuildAllDirty();

        const outputs: ModuleOutput[] = [];
        if (params.showHostVoxels) {
          outputs.push({
            kind: "voxels",
            voxels: {
              positions: compactToVoxelPositions(hostCompact, grid.origin, params.voxelSize),
              voxelSize: params.voxelSize,
              color: [0.3, 0.7, 1.0],
              renderMode: "points",
              pointSize: params.voxelSize * 0.6,
            },
            label: "Host Voxels",
          });
        }
        if (params.showResultVoxels) {
          outputs.push({
            kind: "voxels",
            voxels: {
              positions: compactToVoxelPositions(resultCompact, grid.origin, params.voxelSize),
              voxelSize: params.voxelSize,
              color: [1.0, 0.55, 0.25],
              renderMode: "points",
              pointSize: params.voxelSize * 0.75,
            },
            label: "Lattice Voxels",
          });
        }
        meshOutputsFromChunks(
          outputs,
          rebuild.swappedMeshes,
          params.debugChunkBounds,
          params.debugWireframe,
          params.colorMode,
          params.voxelSize
        );

        const hostCount = countCompactVoxels(hostCompact);
        const resultCount = countCompactVoxels(resultCompact);
        const triangleCount = outputs.reduce((sum, output) => {
          if (output.kind !== "mesh" || !output.mesh.indices) return sum;
          return sum + output.mesh.indices.length / 3;
        }, 0);

        const status = `host ${hostCount} -> lattice ${resultCount} voxels | ${triangleCount} tris`;
        updateStatus?.(status);
        logger?.(`[lattice-chunk-pipeline] ${status}`);

        const overlay = getDebugOverlay();
        overlay?.update("performance", [
          { label: "Host Voxels", value: `${hostCount}` },
          { label: "Lattice Voxels", value: `${resultCount}` },
          { label: "Triangles", value: `${triangleCount}` },
        ]);

        return outputs;
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error);
        updateStatus?.(`Error: ${msg}`);
        logger?.(`[lattice-chunk-pipeline] error: ${msg}`);
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
