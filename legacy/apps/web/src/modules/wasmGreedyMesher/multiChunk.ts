import type { ModuleOutput } from "../types";
import type { ChunkManagerClient } from "./workers/chunkManagerClient";
import { CS, generateBoxWireframe, type MesherParams } from "./helpers";

type MultiChunkState = {
  chunkManagerClient: ChunkManagerClient | null;
  chunkManagerReady: boolean;
  lastVoxelSize: number | null;
  statusText: string;
  updateStatus: ((value: string) => void) | null;
};

type MultiChunkDeps = {
  updatePerformanceOverlay: (genMs: number, meshMs: number, extras?: string) => void;
  updateChunksOverlay: (chunkCount: number, triangles: number, vertices: number, quads?: number) => void;
  updateMemoryOverlay: (
    voxelBytes: number,
    meshBytes: number,
    compressionRatio: number,
    bitsPerVoxel: number
  ) => void;
  clearOverlay: () => void;
};

export const runMultiChunk = async (
  params: MesherParams,
  state: MultiChunkState,
  deps: MultiChunkDeps
): Promise<ModuleOutput[]> => {
  const chunkManagerClient = state.chunkManagerClient;
  if (!chunkManagerClient) {
    state.statusText = "ChunkManager not available";
    state.updateStatus?.(state.statusText);
    return [];
  }

  try {
    if (!state.chunkManagerReady || state.lastVoxelSize !== params.voxelSize) {
      state.updateStatus?.("Initializing chunk manager...");
      await chunkManagerClient.initChunkManager(
        { maxChunksPerFrame: 10000, maxTimeMs: 60000, voxelSize: params.voxelSize },
        { maxBytes: 512 * 1024 * 1024, highWatermark: 0.9, lowWatermark: 0.7, minChunks: 4 }
      );
      state.chunkManagerReady = true;
      state.lastVoxelSize = params.voxelSize;
    }

    state.updateStatus?.("Generating & meshing chunks...");

    const result = await chunkManagerClient.generateAndPopulate({
      gridSize: params.gridSize,
      voxelSize: params.voxelSize,
      pattern: params.pattern,
      simplexScale: params.simplexScale,
      simplexOctaves: params.simplexOctaves,
      simplexThreshold: params.simplexThreshold
    });

    const outputs: ModuleOutput[] = [];
    let totalTriangles = 0;
    let totalVertices = 0;

    for (const mesh of result.swappedMeshes) {
      totalTriangles += mesh.triangleCount;
      totalVertices += mesh.vertexCount;

      outputs.push({
        kind: "mesh",
        mesh: {
          positions: mesh.positions,
          indices: mesh.indices,
          normals: mesh.normals
        },
        label: `Chunk (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`
      });
    }

    if (params.debugChunkBounds && result.swappedMeshes.length > 0) {
      for (const mesh of result.swappedMeshes) {
        const offsetX = mesh.coord.x * CS * params.voxelSize;
        const offsetY = mesh.coord.y * CS * params.voxelSize;
        const offsetZ = mesh.coord.z * CS * params.voxelSize;
        const chunkExtent = CS * params.voxelSize;

        const boundsPositions = generateBoxWireframe(
          offsetX, offsetY, offsetZ,
          offsetX + chunkExtent, offsetY + chunkExtent, offsetZ + chunkExtent
        );
        outputs.push({
          kind: "lines",
          lines: { positions: boundsPositions, color: [0.0, 1.0, 1.0] },
          label: `Bounds (${mesh.coord.x},${mesh.coord.y},${mesh.coord.z})`
        });
      }
    }

    state.statusText = `Chunks: ${result.chunksRebuilt} | Tri: ${totalTriangles} | Vtx: ${totalVertices}`;
    state.updateStatus?.(state.statusText);

    deps.updatePerformanceOverlay(result.genTime, result.meshTime);
    deps.updateChunksOverlay(result.chunksRebuilt, totalTriangles, totalVertices);

    try {
      const debugInfo = await chunkManagerClient.debugInfo();
      deps.updateMemoryOverlay(
        debugInfo.voxelMemoryBytes,
        debugInfo.meshMemoryBytes,
        debugInfo.averageCompressionRatio,
        debugInfo.averageBitsPerVoxel
      );
    } catch {
      // Memory stats unavailable
    }

    return outputs;
  } catch (error) {
    const msg = (error as Error).message;
    if (msg === "superseded") {
      return [];
    }
    state.statusText = `Error: ${msg}`;
    state.updateStatus?.(state.statusText);
    deps.clearOverlay();
    return [];
  }
};
