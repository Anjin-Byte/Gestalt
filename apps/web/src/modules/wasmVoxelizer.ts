import type { ModuleOutput, TestbedModule } from "./types";
import { VoxelizerAdapter } from "@gestalt/voxelizer-js";

type SampleModel = { id: string; label: string; file: string };

const defaultSampleModels: SampleModel[] = [
  { id: "cube", label: "Cube", file: "models/cube.obj" },
  { id: "pyramid", label: "Pyramid", file: "models/pyramid.obj" },
  { id: "bunny", label: "Bunny", file: "models/bunny.obj" },
  { id: "teapot", label: "Teapot", file: "models/teapot.obj" },
  { id: "elephant", label: "Elephant", file: "models/elephant.obj" },
  { id: "dragon", label: "Dragon", file: "models/dragon.obj" },
  { id: "chess-king", label: "Chess King", file: "models/ChessKing.obj" },
  { id: "sponza", label: "Sponza", file: "models/sponza.obj" }
];

const parseObjFallback = (input: string) => {
  const positions: number[] = [];
  const indices: number[] = [];
  const lines = input.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("v ")) {
      const parts = trimmed.split(/\s+/);
      if (parts.length >= 4) {
        const x = Number(parts[1]);
        const y = Number(parts[2]);
        const z = Number(parts[3]);
        if (Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(z)) {
          positions.push(x, y, z);
        }
      }
    } else if (trimmed.startsWith("f ")) {
      const parts = trimmed.split(/\s+/).slice(1);
      const faceIndices = parts
        .map((part) => Number(part.split("/")[0]))
        .filter((value) => Number.isFinite(value) && value > 0)
        .map((value) => value - 1);
      if (faceIndices.length >= 3) {
        const base = faceIndices[0];
        for (let i = 1; i < faceIndices.length - 1; i += 1) {
          indices.push(base, faceIndices[i], faceIndices[i + 1]);
        }
      }
    }
  }
  return {
    positions: new Float32Array(positions),
    indices: new Uint32Array(indices)
  };
};

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

const asNumber = (value: unknown, fallback: number) => {
  const num = Number(value);
  return Number.isFinite(num) ? num : fallback;
};

const asInt = (value: unknown, fallback: number) =>
  Math.floor(asNumber(value, fallback));

const asBool = (value: unknown, fallback: boolean) =>
  typeof value === "boolean" ? value : fallback;

const normalizeRenderMode = (value: unknown) =>
  value === "cubes" || value === "points" ? value : "points";

const normalizeFloat32Array = (input: Float32Array<ArrayBufferLike>) =>
  input.buffer instanceof ArrayBuffer ? input : new Float32Array(input);

type VoxelParams = {
  gridDim: number;
  voxelSize: number;
  epsilon: number;
  fitBounds: boolean;
  progressive: boolean;
  compact: boolean;
  gpuCompact: boolean;
  paging: boolean;
  page: number;
  bricksPerPage: number;
  showBrickBounds: boolean;
  renderMode: "points" | "cubes";
  renderChunk: number;
  pointSize: number;
  chunkSize: number;
  wasmLogs: boolean;
};

const computeAutoRenderChunk = (limits: GPUDevice["limits"] | null) => {
  if (!limits || !limits.maxStorageBufferBindingSize) {
    return null;
  }
  const bytesPerInstance = 16 * 4;
  const safety = 0.25;
  const maxInstances = Math.floor(
    (limits.maxStorageBufferBindingSize * safety) / bytesPerInstance
  );
  return clamp(maxInstances, 1000, 5_000_000);
};

const appendBrickBoundsLines = (
  lines: number[],
  brickOrigin: [number, number, number],
  brickDim: number,
  voxelSize: number,
  origin: [number, number, number]
) => {
  const minX = origin[0] + brickOrigin[0] * voxelSize;
  const minY = origin[1] + brickOrigin[1] * voxelSize;
  const minZ = origin[2] + brickOrigin[2] * voxelSize;
  const size = brickDim * voxelSize;
  const maxX = minX + size;
  const maxY = minY + size;
  const maxZ = minZ + size;
  const corners: [number, number, number][] = [
    [minX, minY, minZ],
    [maxX, minY, minZ],
    [maxX, maxY, minZ],
    [minX, maxY, minZ],
    [minX, minY, maxZ],
    [maxX, minY, maxZ],
    [maxX, maxY, maxZ],
    [minX, maxY, maxZ]
  ];
  const edges = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 0],
    [4, 5],
    [5, 6],
    [6, 7],
    [7, 4],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7]
  ];
  for (const [a, b] of edges) {
    lines.push(corners[a][0], corners[a][1], corners[a][2]);
    lines.push(corners[b][0], corners[b][1], corners[b][2]);
  }
};

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

  const loadSampleModel = async (id: string) => {
    const entry = sampleModels.find((model) => model.id === id) ?? sampleModels[0];
    if (!entry) {
      sampleText = "";
      return;
    }
    const cached = sampleCache.get(entry.id);
    if (cached) {
      sampleText = cached;
      return;
    }
    if (!ctxRef) {
      sampleText = "";
      return;
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
          loadWasm: async () => await import("../wasm/wasm_voxelizer/wasm_voxelizer.js"),
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
      const previewProgress = params.renderMode === "points";
      logStage(
        "params",
        `grid=${params.gridDim} voxel=${params.voxelSize.toFixed(4)} ` +
          `render=${params.renderMode} chunkSize=${params.chunkSize} ` +
          `renderChunk=${effectiveRenderChunk} progressive=${params.progressive} ` +
          `compact=${params.compact} gpuCompact=${params.gpuCompact} ` +
          `paging=${params.paging}`
      );

      try {
        const selectedLabel = String(job.params["sample-model"] ?? sampleModels[0]?.label ?? "");
        const selectedModel =
          sampleModels.find((model) => model.label === selectedLabel) ?? sampleModels[0];
        const selectedId = selectedModel?.id ?? sampleId;
        if (!hasUploadedFile && selectedId !== sampleId) {
          sampleId = selectedId;
          sampleText = "";
        }
        if (!hasUploadedFile && !sampleText) {
          await loadSampleModel(sampleId);
        }
        const sourceText = hasUploadedFile ? objText : sampleText;
        if (!sourceText) {
          statusText = "No sample model available";
          updateStatus?.(statusText);
          logStage("prep", "no sample model available");
          return [];
        }
        const { positions, indices } = parseObjFallback(sourceText);
        if (positions.length === 0 || indices.length === 0) {
          statusText = "No faces found";
          updateStatus?.(statusText);
          logStage("prep", "no faces found");
          return [];
        }

        const resolved = voxelizer.resolveGrid({
          positions,
          gridDim: params.gridDim,
          voxelSize: params.voxelSize,
          fitBounds: params.fitBounds
        });
        const { grid, voxelSize, origin } = resolved;
        let voxels: Float32Array<ArrayBufferLike> = new Float32Array();
        let brickDim = 0;
        let brickCount = 0;

        logStage("voxelize", `grid=${params.gridDim} voxel=${voxelSize.toFixed(4)} fit=${params.fitBounds}`);

        const denseVoxels = params.gridDim * params.gridDim * params.gridDim;
        const maxPositionsDevice = Math.floor(
          (deviceLimits?.maxStorageBufferBindingSize ?? 0) / 16
        );
        const maxPositions = Math.max(
          1,
          Math.min(denseVoxels, maxPositionsDevice || denseVoxels)
        );

        if (params.paging) {
          if (params.gpuCompact) {
            logStage("params", "paging enabled: gpuCompact ignored (needs occupancy)");
          }
          const chunks = await voxelizer.voxelizeSparseChunked({
            positions,
            indices,
            grid,
            epsilon: params.epsilon,
            chunkSize: params.chunkSize,
            compact: params.compact
          });
          const bricks = voxelizer.flattenBricksFromChunks(chunks);
          const pageResult = voxelizer.pageBricks(bricks, {
            page: params.page,
            bricksPerPage: params.bricksPerPage
          });
          const active = pageResult.bricks;
          voxels = voxelizer.buildPositionsForBricks(active, voxelSize, origin);
          const boundsLines: number[] = [];
          if (params.showBrickBounds) {
            for (const brick of active) {
              appendBrickBoundsLines(
                boundsLines,
                brick.origin,
                brick.brickDim,
                voxelSize,
                origin
              );
            }
          }
          brickCount = bricks.length;
          brickDim = chunks[0]?.brick_dim ?? 0;
          updatePaging?.(
            pageResult.totalPages > 0
              ? `page ${pageResult.page + 1}/${pageResult.totalPages} bricks ${active.length}/${bricks.length}` +
                  (params.bricksPerPage === 0 ? " (auto)" : "")
              : "no bricks"
          );

          const outputs: ModuleOutput[] = [];
          outputs.push({
            kind: "voxels",
            voxels: {
              positions: normalizeFloat32Array(voxels),
              voxelSize,
              color: [0.4, 0.9, 0.6],
              renderMode: params.renderMode,
              chunkSize: effectiveRenderChunk,
              pointSize: params.pointSize
            },
            label: "Voxelized (paged)"
          });
          if (params.showBrickBounds && boundsLines.length > 0) {
            outputs.push({
              kind: "lines",
              lines: {
                positions: new Float32Array(boundsLines),
                color: [0.9, 0.5, 0.2]
              },
              label: "Brick Bounds"
            });
          }

          const voxelCount = voxels.length / 3;
          const density = denseVoxels > 0 ? (voxelCount / denseVoxels) * 100 : 0;
          updateSparseStats?.(
            `bricks=${brickCount} voxels=${Math.round(voxelCount)} density=${density.toFixed(2)}%`
          );
          updateBrickDim?.(`${brickDim}`);
          statusText = `Paged voxels: ${Math.round(voxelCount)}`;
          updateStatus?.(statusText);
          logStage(
            "run",
            `complete paging voxels=${Math.round(voxelCount)} ms=${(
              performance.now() - runStart
            ).toFixed(1)}`
          );
          return outputs;
        }

        updatePaging?.("Disabled");

        if (params.progressive && !params.gpuCompact) {
          const chunks = await voxelizer.voxelizeSparseChunked({
            positions,
            indices,
            grid,
            epsilon: params.epsilon,
            chunkSize: params.chunkSize,
            compact: params.compact
          });
          logStage("voxelize", `chunks=${chunks.length}`);
          const allPositions: number[] = [];
          for (const chunk of chunks) {
            const chunkVoxels = voxelizer.expandSparseToPositions(chunk, origin, voxelSize);
            for (let i = 0; i < chunkVoxels.length; i += 1) {
              allPositions.push(chunkVoxels[i]);
            }
            brickDim = chunk.brick_dim;
            brickCount += Math.floor(chunk.brick_origins.length / 3);
            if (previewProgress) {
              ctxRef?.emitOutputs?.([
                {
                  kind: "voxels",
                  voxels: {
                    positions: new Float32Array(allPositions),
                    voxelSize,
                    color: [0.4, 0.9, 0.6],
                    renderMode: params.renderMode,
                    chunkSize: effectiveRenderChunk,
                    pointSize: params.pointSize
                  },
                  label: "Voxelized (progressive)"
                }
              ]);
            }
            await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
          }
          voxels = new Float32Array(allPositions);
        } else if (params.progressive && params.gpuCompact) {
          const chunks = await voxelizer.voxelizePositionsChunked({
            positions,
            indices,
            grid,
            epsilon: params.epsilon,
            chunkSize: params.chunkSize,
            maxPositions
          });
          const allPositions: number[] = [];
          for (const chunk of chunks) {
            for (let i = 0; i < chunk.positions.length; i += 1) {
              allPositions.push(chunk.positions[i]);
            }
            brickDim = chunk.brick_dim;
            brickCount += chunk.brick_count;
            if (previewProgress) {
              ctxRef?.emitOutputs?.([
                {
                  kind: "voxels",
                  voxels: {
                    positions: new Float32Array(allPositions),
                    voxelSize,
                    color: [0.4, 0.9, 0.6],
                    renderMode: params.renderMode,
                    chunkSize: effectiveRenderChunk,
                    pointSize: params.pointSize
                  },
                  label: "Voxelized (gpu-compact progressive)"
                }
              ]);
            }
            await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
          }
          voxels = new Float32Array(allPositions);
        } else if (params.gpuCompact) {
          const result = await voxelizer.voxelizePositions({
            positions,
            indices,
            grid,
            epsilon: params.epsilon,
            maxPositions
          });
          voxels = new Float32Array(result.positions);
          brickDim = result.brick_dim;
          brickCount = result.brick_count;
        } else {
          const result = await voxelizer.voxelizeSparse({
            positions,
            indices,
            grid,
            epsilon: params.epsilon
          });
          voxels = voxelizer.expandSparseToPositions(result, origin, voxelSize);
          brickDim = result.brick_dim;
          brickCount = Math.floor(result.brick_origins.length / 3);
        }

        const voxelCount = voxels.length / 3;
        const density = denseVoxels > 0 ? (voxelCount / denseVoxels) * 100 : 0;
        updateSparseStats?.(
          `bricks=${brickCount} voxels=${Math.round(voxelCount)} density=${density.toFixed(2)}%`
        );
        updateBrickDim?.(`${brickDim}`);

        statusText = `Voxels: ${Math.round(voxelCount)}`;
        updateStatus?.(statusText);
        logStage("run", `complete voxels=${Math.round(voxelCount)} ms=${(performance.now() - runStart).toFixed(1)}`);

        const output: ModuleOutput = {
          kind: "voxels",
          voxels: {
            positions: normalizeFloat32Array(voxels),
            voxelSize,
            color: [0.4, 0.9, 0.6],
            renderMode: params.renderMode,
            chunkSize: effectiveRenderChunk,
            pointSize: params.pointSize
          },
          label: "Voxelized"
        };
        return [output];
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
