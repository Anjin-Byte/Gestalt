import type { DebugColorMode } from "./workers/mesherTypes";
import {
  getDebugOverlay,
  formatBytes,
  formatMs,
  formatPercent,
  formatCount
} from "../../ui/debugOverlay";

export const CS = 62;

export type MesherParams = {
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

export const DIR_LABELS = ["+Y", "-Y", "+X", "-X", "+Z", "-Z"];

export const generateBoxWireframe = (
  minX: number, minY: number, minZ: number,
  maxX: number, maxY: number, maxZ: number
): Float32Array => {
  const c = [
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
    [0, 1], [1, 2], [2, 3], [3, 0],
    [4, 5], [5, 6], [6, 7], [7, 4],
    [0, 4], [1, 5], [2, 6], [3, 7]
  ];

  const positions = new Float32Array(72);
  let idx = 0;
  for (const [a, b] of edges) {
    positions[idx++] = c[a][0]; positions[idx++] = c[a][1]; positions[idx++] = c[a][2];
    positions[idx++] = c[b][0]; positions[idx++] = c[b][1]; positions[idx++] = c[b][2];
  }
  return positions;
};

export const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

export const asNumber = (value: unknown, fallback: number): number => {
  const num = Number(value);
  return Number.isFinite(num) ? num : fallback;
};

export const asInt = (value: unknown, fallback: number): number =>
  Math.floor(asNumber(value, fallback));

export const asBool = (value: unknown, fallback: boolean): boolean =>
  typeof value === "boolean" ? value : fallback;

export const asString = (value: unknown, fallback: string): string =>
  typeof value === "string" ? value : fallback;

export const updatePerformanceOverlay = (genMs: number, meshMs: number, extras?: string) => {
  const overlay = getDebugOverlay();
  if (!overlay) return;
  const entries = [
    { label: "Generate", value: formatMs(genMs) },
    { label: "Mesh", value: formatMs(meshMs) }
  ];
  if (extras) {
    entries.push({ label: "Info", value: extras });
  }
  overlay.update("performance", entries);
};

export const updateMemoryOverlay = (
  voxelBytes: number,
  meshBytes: number,
  compressionRatio: number,
  bitsPerVoxel: number
) => {
  const overlay = getDebugOverlay();
  if (!overlay) return;
  overlay.update("memory", [
    { label: "Voxel", value: formatBytes(voxelBytes) },
    { label: "Mesh", value: formatBytes(meshBytes) },
    { label: "Total", value: formatBytes(voxelBytes + meshBytes) },
    { label: "Compression", value: formatPercent(compressionRatio) },
    { label: "Bits/Voxel", value: bitsPerVoxel.toFixed(1) }
  ]);
};

export const updateChunksOverlay = (
  chunkCount: number,
  triangles: number,
  vertices: number,
  quads?: number
) => {
  const overlay = getDebugOverlay();
  if (!overlay) return;
  const entries = [
    { label: "Chunks", value: String(chunkCount) },
    { label: "Triangles", value: formatCount(triangles) },
    { label: "Vertices", value: formatCount(vertices) }
  ];
  if (quads !== undefined) {
    entries.push({ label: "Quads", value: formatCount(quads) });
  }
  overlay.update("chunks", entries);
};

export const clearOverlay = () => {
  const overlay = getDebugOverlay();
  overlay?.clearAll();
};
