import type { VoxelizerAdapter } from "@gestalt/voxelizer-js";
import type { ModuleOutput, RunRequest } from "../types";
import {
  appendBrickBoundsLines,
  normalizeFloat32Array,
  parseObjFallback,
  type SampleModel,
  type VoxelParams
} from "./helpers";

type VoxelizerRunCoreArgs = {
  voxelizer: VoxelizerAdapter;
  params: VoxelParams;
  effectiveRenderChunk: number;
  job: RunRequest;
  runStart: number;
  sampleModels: SampleModel[];
  sampleId: string;
  sampleText: string;
  hasUploadedFile: boolean;
  objText: string;
  deviceLimits: GPUDevice["limits"] | null;
  ctxRef: {
    requestGpuDevice: () => Promise<GPUDevice | null>;
    emitOutputs?: (outputs: ModuleOutput[]) => void;
    baseUrl: string;
  } | null;
  updateSparseStats: ((value: string) => void) | null;
  updateBrickDim: ((value: string) => void) | null;
  updatePaging: ((value: string) => void) | null;
  logStage: (stage: string, message: string) => void;
  loadSampleModel: (id: string) => Promise<string>;
  setStatus: (value: string) => void;
};

type VoxelizerRunCoreResult = {
  outputs: ModuleOutput[];
  sampleId: string;
  sampleText: string;
};

export const runVoxelizerCore = async (
  args: VoxelizerRunCoreArgs
): Promise<VoxelizerRunCoreResult> => {
  let sampleId = args.sampleId;
  let sampleText = args.sampleText;
  const {
    voxelizer,
    params,
    effectiveRenderChunk,
    job,
    runStart,
    sampleModels,
    hasUploadedFile,
    objText,
    deviceLimits,
    ctxRef,
    updateSparseStats,
    updateBrickDim,
    updatePaging,
    logStage,
    loadSampleModel,
    setStatus
  } = args;

  const selectedLabel = String(job.params["sample-model"] ?? sampleModels[0]?.label ?? "");
  const selectedModel =
    sampleModels.find((model) => model.label === selectedLabel) ?? sampleModels[0];
  const selectedId = selectedModel?.id ?? sampleId;
  if (!hasUploadedFile && selectedId !== sampleId) {
    sampleId = selectedId;
    sampleText = "";
  }
  if (!hasUploadedFile && !sampleText) {
    sampleText = await loadSampleModel(sampleId);
  }
  const sourceText = hasUploadedFile ? objText : sampleText;
  if (!sourceText) {
    setStatus("No sample model available");
    logStage("prep", "no sample model available");
    return { outputs: [], sampleId, sampleText };
  }
  const { positions, indices } = parseObjFallback(sourceText);
  if (positions.length === 0 || indices.length === 0) {
    setStatus("No faces found");
    logStage("prep", "no faces found");
    return { outputs: [], sampleId, sampleText };
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
    setStatus(`Paged voxels: ${Math.round(voxelCount)}`);
    logStage(
      "run",
      `complete paging voxels=${Math.round(voxelCount)} ms=${(
        performance.now() - runStart
      ).toFixed(1)}`
    );
    return { outputs, sampleId, sampleText };
  }

  updatePaging?.("Disabled");
  const previewProgress = params.renderMode === "points";

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

  setStatus(`Voxels: ${Math.round(voxelCount)}`);
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
  return { outputs: [output], sampleId, sampleText };
};
