/**
 * Voxel Chunk Pipeline module (ADR-0009).
 *
 * Demonstrates the full OBJ -> GPU voxelize -> ChunkManager -> greedy mesh pipeline.
 * Loads an OBJ file, voxelizes it on the GPU with material resolution, ingests
 * compact voxels into the chunk system, and renders greedy-meshed output.
 */

import type { TestbedModule, ModuleContext } from "../types";
import { MesherClient } from "../wasmGreedyMesher/workers/mesherClient";
import { ChunkManagerClient } from "../wasmGreedyMesher/workers/chunkManagerClient";
import { parseObjFallback, buildMaterialTable } from "../wasmObjLoader/helpers";
import { voxelizeToChunks } from "../wasmGreedyMesher/voxelizeToChunks";
import { getDebugOverlay } from "../../ui/debugOverlay";
import { defaultSampleModels, type SampleModel } from "../wasmVoxelizer/helpers";

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

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);
const asNumber = (v: unknown, fallback: number) => {
  const n = Number(v);
  return Number.isFinite(n) ? n : fallback;
};
const asInt = (v: unknown, fallback: number) => Math.round(asNumber(v, fallback));
const asBool = (v: unknown, fallback: boolean) =>
  typeof v === "boolean" ? v : fallback;
const asString = (v: unknown, fallback: string) =>
  typeof v === "string" ? v : fallback;

/** Compute bounds of a Float32Array of xyz triples. */
const computeBounds = (positions: Float32Array) => {
  let minX = Infinity, minY = Infinity, minZ = Infinity;
  let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;
  for (let i = 0; i < positions.length; i += 3) {
    const x = positions[i], y = positions[i + 1], z = positions[i + 2];
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (z < minZ) minZ = z;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
    if (z > maxZ) maxZ = z;
  }
  if (!Number.isFinite(minX)) return null;
  return {
    min: [minX, minY, minZ] as [number, number, number],
    max: [maxX, maxY, maxZ] as [number, number, number],
  };
};

export const createVoxelChunkPipelineModule = (): TestbedModule => {
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

  return {
    id: "voxel-chunk-pipeline",
    name: "Voxel Chunk Pipeline",

    init: async (ctx: ModuleContext) => {
      logger = ctx.logger.info;
      ctxRef = ctx;
      logger("[voxel-chunk-pipeline] init start");
      try {
        await Promise.all([ensureVoxelizer(), ensureMesher()]);
        // Pre-load the default sample model so the first run is instant
        sampleText = await loadSampleModel(sampleId);
        if (sampleText) {
          fileName = sampleModels.find((m) => m.id === sampleId)?.label ?? sampleId;
          updateFileName?.(fileName);
        }
        statusText = "Loaded";
        updateStatus?.(statusText);
        logger!("[voxel-chunk-pipeline] init complete");
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(`[voxel-chunk-pipeline] init failed: ${(error as Error).message}`);
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

      api.addNumber({
        id: "grid-dim",
        label: "Grid Dim",
        min: 8,
        max: 4096,
        step: 8,
        initial: 1024,
      });
      api.addNumber({
        id: "voxel-size",
        label: "Voxel Size",
        min: 0.001,
        max: 0.5,
        step: 0.001,
        initial: 0.1,
      });
      api.addNumber({
        id: "epsilon",
        label: "Epsilon",
        min: 0,
        max: 0.01,
        step: 0.0001,
        initial: 0.0001,
      });
      api.addCheckbox({ id: "fit-bounds", label: "Fit Grid To Mesh", initial: true });
      api.addSelect({
        id: "color-mode",
        label: "Color Mode",
        options: ["none", "material", "chunk", "face-direction", "quad-size"],
        initial: "quad-size",
      });
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

      // Resolve model source: uploaded file takes priority over sample selector
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

      const gridDim = clamp(asInt(job.params["grid-dim"], 128), 8, 2048);
      let voxelSize = clamp(asNumber(job.params["voxel-size"], 0.1), 0.001, 0.5);
      const epsilon = clamp(asNumber(job.params.epsilon, 0.0001), 0, 0.01);
      const fitBounds = asBool(job.params["fit-bounds"], true);
      const debugChunkBounds = asBool(job.params["debug-chunk-bounds"], false);
      const debugWireframe = asBool(job.params["debug-wireframe"], false);
      const colorMode = asString(job.params["color-mode"], "material") as "none" | "material" | "chunk" | "face-direction" | "quad-size";

      // Parse OBJ
      updateStatus?.("Parsing OBJ...");
      const parsed = parseObjFallback(sourceText);
      if (parsed.positions.length === 0 || parsed.indices.length === 0) {
        updateStatus?.("No faces found in OBJ");
        return [];
      }
      logger?.(`[voxel-chunk-pipeline] parsed: ${parsed.positions.length / 3} verts, ${parsed.indices.length / 3} tris`);

      // Compute grid origin + voxel size
      let origin: [number, number, number];
      if (fitBounds) {
        const bounds = computeBounds(parsed.positions);
        if (bounds) {
          const size = [
            bounds.max[0] - bounds.min[0],
            bounds.max[1] - bounds.min[1],
            bounds.max[2] - bounds.min[2],
          ];
          const extent = Math.max(size[0], size[1], size[2]) || 1;
          voxelSize = extent / gridDim;
          const half = extent * 0.5;
          const center: [number, number, number] = [
            (bounds.min[0] + bounds.max[0]) * 0.5,
            (bounds.min[1] + bounds.max[1]) * 0.5,
            (bounds.min[2] + bounds.max[2]) * 0.5,
          ];
          origin = [center[0] - half, center[1] - half, center[2] - half];
        } else {
          origin = [0, 0, 0];
        }
      } else {
        const half = gridDim * voxelSize * 0.5;
        origin = [-half, -half, -half];
      }

      // Build material table
      const materialTable = buildMaterialTable(parsed.triangleMaterials, parsed.materialGroupNames);

      // Always re-init chunk manager for a clean state.
      // Creating a fresh WasmChunkManager is cheap compared to voxelization,
      // and avoids stale state issues with fire-and-forget clear().
      updateStatus?.("Initializing chunk manager...");
      await chunkManagerClient.initChunkManager(
        { maxChunksPerFrame: 10000, maxTimeMs: 60000, voxelSize },
        { maxBytes: 512 * 1024 * 1024, highWatermark: 0.9, lowWatermark: 0.7, minChunks: 4 },
      );

      // Run the pipeline
      updateStatus?.("Voxelizing (GPU)...");
      const t0 = performance.now();

      try {
        const outputs = await voxelizeToChunks(voxelizer, chunkManagerClient, {
          positions: parsed.positions,
          indices: parsed.indices,
          materialTable,
          origin,
          voxelSize,
          dims: [gridDim, gridDim, gridDim],
          epsilon,
          debugChunkBounds,
          debugWireframe,
          colorMode,
        });

        const elapsed = (performance.now() - t0).toFixed(0);
        const meshCount = outputs.filter((o) => o.kind === "mesh").length;
        let totalTri = 0;
        for (const o of outputs) {
          if (o.kind === "mesh" && o.mesh.indices) {
            totalTri += o.mesh.indices.length / 3;
          }
        }

        const status = `${meshCount} chunks | ${totalTri} tris | ${elapsed}ms`;
        updateStatus?.(status);
        logger?.(`[voxel-chunk-pipeline] done: ${status}`);

        // Update debug overlay
        const overlay = getDebugOverlay();
        overlay?.update("performance", [
          { label: "Pipeline", value: `${elapsed}ms` },
          { label: "Chunks", value: `${meshCount}` },
          { label: "Triangles", value: `${totalTri}` },
        ]);

        return outputs;
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error);
        updateStatus?.(`Error: ${msg}`);
        logger?.(`[voxel-chunk-pipeline] error: ${msg}`);
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
