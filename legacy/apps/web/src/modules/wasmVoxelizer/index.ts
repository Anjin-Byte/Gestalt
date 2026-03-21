import type { ModuleOutput, TestbedModule } from "../types";
import { VoxelizerAdapter } from "@gestalt/voxelizer-js";
import {
  asBool,
  asInt,
  asNumber,
  clamp,
  computeAutoRenderChunk,
  defaultSampleModels,
  normalizeRenderMode,
  type SampleModel,
  type VoxelParams
} from "./helpers";
import { runVoxelizerCore } from "./runCore";

export const createWasmVoxelizerModule = (): TestbedModule => {
  let voxelizer: VoxelizerAdapter | null = null;
  let statusText = "Not loaded";
  let updateStatus: ((value: string) => void) | null = null;
  let updateLimits: ((value: string) => void) | null = null;
  let updateMaxGrid: ((value: string) => void) | null = null;
  let updateBrickDim: ((value: string) => void) | null = null;
  let updateSparseStats: ((value: string) => void) | null = null;
  let updatePaging: ((value: string) => void) | null = null;
  let updateFileName: ((value: string) => void) | null = null;
  let fileName = "No file";
  let objText = "";
  let sampleModels: SampleModel[] = [...defaultSampleModels];
  let sampleId = sampleModels[0]?.id ?? "cube";
  let sampleText = "";
  let hasUploadedFile = false;
  const sampleCache = new Map<string, string>();
  let logEnabled = true;
  let runInFlight = false;
  let deviceLimits: GPUDevice["limits"] | null = null;
  let ctxRef: {
    requestGpuDevice: () => Promise<GPUDevice | null>;
    emitOutputs?: (outputs: ModuleOutput[]) => void;
    baseUrl: string;
  } | null = null;
  let logger: ((message: string) => void) | null = null;

  const logStage = (stage: string, message: string) => {
    logger?.(`[wasm-voxelizer:${stage}] ${message}`);
  };

  const loadSampleModel = async (id: string): Promise<string> => {
    const entry = sampleModels.find((model) => model.id === id) ?? sampleModels[0];
    if (!entry) {
      sampleText = "";
      return sampleText;
    }
    const cached = sampleCache.get(entry.id);
    if (cached) {
      sampleText = cached;
      return sampleText;
    }
    if (!ctxRef) {
      sampleText = "";
      return sampleText;
    }
    const base = new URL(ctxRef.baseUrl || "/", window.location.href);
    const url = new URL(entry.file, base).toString();
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`Sample fetch failed: ${entry.label}`);
    }
    const text = await response.text();
    sampleCache.set(entry.id, text);
    sampleText = text;
    return sampleText;
  };

  const loadSampleManifest = async (baseUrl: string) => {
    const base = new URL(baseUrl || "/", window.location.href);
    const url = new URL("models/index.json", base).toString();
    const response = await fetch(url);
    if (!response.ok) {
      return defaultSampleModels;
    }
    const data = await response.json();
    if (!Array.isArray(data)) {
      return defaultSampleModels;
    }
    const models = data
      .map((entry) => {
        if (!entry || typeof entry !== "object") {
          return null;
        }
        const id = String((entry as { id?: unknown }).id ?? "").trim();
        const label = String((entry as { label?: unknown }).label ?? "").trim();
        const file = String((entry as { file?: unknown }).file ?? "").trim();
        if (!id || !label || !file) {
          return null;
        }
        return { id, label, file };
      })
      .filter((entry): entry is SampleModel => Boolean(entry));
    return models.length > 0 ? models : defaultSampleModels;
  };

  const computeMaxGridDim = () => {
    if (!deviceLimits) {
      return null;
    }
    const maxInvocations = deviceLimits.maxComputeInvocationsPerWorkgroup;
    const maxTile = Math.floor(Math.cbrt(maxInvocations));
    const maxStorage = deviceLimits.maxStorageBufferBindingSize;
    const maxVoxels = Math.floor(maxStorage / 4);
    const maxGrid = Math.floor(Math.cbrt(maxVoxels));
    return { maxTileDim: Math.max(1, maxTile), maxGridDim: Math.max(1, maxGrid) };
  };

  return {
    id: "wasm-voxelizer",
    name: "WASM Voxelizer",
    init: async (ctx) => {
      logger = ctx.logger.info;
      ctxRef = ctx;
      logStage("init", "start");
      try {
        sampleModels = await loadSampleManifest(ctx.baseUrl ?? "/");
        sampleId = sampleModels[0]?.id ?? "cube";
        voxelizer = await VoxelizerAdapter.create({
          loadWasm: async () => await import("../../wasm/wasm_voxelizer/wasm_voxelizer.js"),
          logEnabled
        });
        const device = await ctx.requestGpuDevice();
        deviceLimits = device?.limits ?? null;
        if (deviceLimits) {
          updateLimits?.(
            `maxInvocations=${deviceLimits.maxComputeInvocationsPerWorkgroup}, ` +
              `maxStorageMB=${Math.round(
                deviceLimits.maxStorageBufferBindingSize / (1024 * 1024)
              )}`
          );
        } else {
          updateLimits?.("WebGPU unavailable");
        }
        statusText = voxelizer ? "Loaded" : "Missing exports";
        updateStatus?.(statusText);
        logStage("init", `complete (ok=${Boolean(voxelizer)})`);
      } catch (error) {
        statusText = "Missing (run pnpm build:wasm)";
        updateStatus?.(statusText);
        ctx.logger.warn(`WASM voxelizer failed to load: ${(error as Error).message}`);
      }
    },
    ui: (api) => {
      api.addText({ id: "wasm-status", label: "Status", initial: statusText });
      api.addText({ id: "device-limits", label: "Device Limits", initial: "Pending" });
      api.addText({ id: "max-grid", label: "Max Grid (dense est)", initial: "Pending" });
      api.addText({ id: "brick-dim", label: "Brick Dim", initial: "Pending" });
      api.addText({ id: "sparse-stats", label: "Sparse Stats", initial: "Pending" });
      api.addText({ id: "paging-status", label: "Paging", initial: "Disabled" });
      api.addText({ id: "obj-file", label: "OBJ File", initial: fileName });
      api.addSelect({
        id: "sample-model",
        label: "Sample Model",
        options: sampleModels.map((model) => model.label),
        initial: sampleModels[0]?.label ?? "Sample"
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
            statusText = "No file selected";
            updateFileName?.(fileName);
            updateStatus?.(statusText);
            return;
          }
          fileName = file.name;
          statusText = "Loading...";
          updateFileName?.(fileName);
          updateStatus?.(statusText);
          try {
            objText = await file.text();
            hasUploadedFile = true;
            statusText = `Ready (${objText.length} chars)`;
            updateStatus?.(statusText);
          } catch {
            statusText = "File read failed";
            updateStatus?.(statusText);
            objText = "";
          }
        }
      });
      api.addCheckbox({ id: "fit-bounds", label: "Fit Grid To Mesh", initial: true });
      api.addCheckbox({ id: "progressive", label: "Progressive Chunks", initial: true });
      api.addCheckbox({ id: "compact", label: "Compact Chunks", initial: true });
      api.addCheckbox({ id: "wasm-logs", label: "WASM Logs", initial: true });
      api.addCheckbox({ id: "gpu-compact", label: "GPU Compact Positions", initial: false });
      api.addCheckbox({ id: "paging", label: "Paging (brick streaming)", initial: false });
      api.addCheckbox({ id: "brick-bounds", label: "Show Brick Bounds", initial: false });
      api.addSelect({
        id: "voxel-render",
        label: "Voxel Render",
        options: ["points", "cubes"],
        initial: "points"
      });
      api.addNumber({
        id: "render-chunk",
        label: "Render Chunk (voxels)",
        min: 0,
        max: 5000000,
        step: 1000,
        initial: 0
      });
      api.addNumber({
        id: "point-size",
        label: "Point Size",
        min: 0.001,
        max: 0.1,
        step: 0.001,
        initial: 0.01
      });
      api.addNumber({
        id: "voxel-size",
        label: "Voxel Size",
        min: 0.0005,
        max: 0.5,
        step: 0.0005,
        initial: 0.1
      });
      api.addNumber({
        id: "grid-dim",
        label: "Grid Dim",
        min: 8,
        max: 1200,
        step: 8,
        initial: 512
      });
      api.addNumber({
        id: "chunk-size",
        label: "Chunk Size (bricks, 0=auto)",
        min: 0,
        max: 512,
        step: 1,
        initial: 0
      });
      api.addNumber({
        id: "epsilon",
        label: "Epsilon",
        min: 0.0,
        max: 0.01,
        step: 0.0005,
        initial: 0.001
      });
      api.addNumber({
        id: "page",
        label: "Page (0=first)",
        min: 0,
        max: 100000,
        step: 1,
        initial: 0
      });
      api.addNumber({
        id: "bricks-per-page",
        label: "Bricks Per Page (0=auto)",
        min: 0,
        max: 500000,
        step: 1,
        initial: 0
      });
      updateStatus = (value: string) => api.setText("wasm-status", value);
      updateLimits = (value: string) => api.setText("device-limits", value);
      updateMaxGrid = (value: string) => api.setText("max-grid", value);
      updateBrickDim = (value: string) => api.setText("brick-dim", value);
      updateSparseStats = (value: string) => api.setText("sparse-stats", value);
      updatePaging = (value: string) => api.setText("paging-status", value);
      updateFileName = (value: string) => api.setText("obj-file", value);
    },
    run: async (job) => {
      if (runInFlight) {
        logStage("run", "skipped (in flight)");
        return [];
      }
      runInFlight = true;
      logStage("run", `start frame=${job.frameId}`);
      const runStart = performance.now();
      if (!voxelizer) {
        statusText = "Not loaded";
        updateStatus?.(statusText);
        logStage("run", "skipped (module not initialized)");
        runInFlight = false;
        return [];
      }

      const gridDim = clamp(asInt(job.params["grid-dim"], 1024), 8, 2048);
      if (!deviceLimits && ctxRef) {
        const device = await ctxRef.requestGpuDevice();
        deviceLimits = device?.limits ?? null;
      }
      const limits = computeMaxGridDim();
      if (limits) {
        updateMaxGrid?.(`~${limits.maxGridDim}^3 voxels`);
        updateBrickDim?.(`${limits.maxTileDim}`);
      }

      const params: VoxelParams = {
        gridDim,
        voxelSize: clamp(asNumber(job.params["voxel-size"], 0.1), 0.0005, 1),
        epsilon: clamp(asNumber(job.params.epsilon, 0.001), 0, 0.01),
        fitBounds: asBool(job.params["fit-bounds"], true),
        progressive: asBool(job.params.progressive, true),
        compact: asBool(job.params.compact, true),
        gpuCompact: asBool(job.params["gpu-compact"], false),
        paging: asBool(job.params.paging, false),
        page: clamp(asInt(job.params.page, 0), 0, 100000),
        bricksPerPage: clamp(asInt(job.params["bricks-per-page"], 0), 0, 500000),
        showBrickBounds: asBool(job.params["brick-bounds"], false),
        renderMode: normalizeRenderMode(job.params["voxel-render"]),
        renderChunk: clamp(asInt(job.params["render-chunk"], 0), 0, 5_000_000),
        pointSize: clamp(asNumber(job.params["point-size"], 0.01), 0.0005, 0.2),
        chunkSize: clamp(asInt(job.params["chunk-size"], 0), 0, 8192),
        wasmLogs: asBool(job.params["wasm-logs"], true)
      };
      const autoRenderChunk = computeAutoRenderChunk(deviceLimits);
      const effectiveRenderChunk =
        params.renderChunk === 0
          ? autoRenderChunk ?? 20000
          : autoRenderChunk
            ? Math.min(params.renderChunk, autoRenderChunk)
            : params.renderChunk;
      if (autoRenderChunk && effectiveRenderChunk !== params.renderChunk && params.renderChunk !== 0) {
        logStage(
          "params",
          `renderChunk clamped ${params.renderChunk} -> ${effectiveRenderChunk}`
        );
      }
      if (params.renderChunk === 0) {
        logStage("params", `renderChunk auto=${effectiveRenderChunk}`);
      }
      logEnabled = params.wasmLogs;
      voxelizer?.setLogEnabled(logEnabled);
      logStage(
        "params",
        `grid=${params.gridDim} voxel=${params.voxelSize.toFixed(4)} ` +
          `render=${params.renderMode} chunkSize=${params.chunkSize} ` +
          `renderChunk=${effectiveRenderChunk} progressive=${params.progressive} ` +
          `compact=${params.compact} gpuCompact=${params.gpuCompact} ` +
          `paging=${params.paging}`
      );

      try {
        const result = await runVoxelizerCore({
          voxelizer,
          params,
          effectiveRenderChunk,
          job,
          runStart,
          sampleModels,
          sampleId,
          sampleText,
          hasUploadedFile,
          objText,
          deviceLimits,
          ctxRef,
          updateSparseStats,
          updateBrickDim,
          updatePaging,
          logStage,
          loadSampleModel,
          setStatus: (value) => {
            statusText = value;
            updateStatus?.(statusText);
          }
        });
        sampleId = result.sampleId;
        sampleText = result.sampleText;
        return result.outputs;
      } catch (error) {
        statusText = "Voxelization failed";
        updateStatus?.(statusText);
        logStage("error", (error as Error).message);
        return [];
      } finally {
        runInFlight = false;
      }
    }
  };
};
