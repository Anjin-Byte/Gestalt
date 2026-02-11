export type SampleModel = { id: string; label: string; file: string };

export const defaultSampleModels: SampleModel[] = [
  { id: "cube", label: "Cube", file: "models/cube.obj" },
  { id: "pyramid", label: "Pyramid", file: "models/pyramid.obj" },
  { id: "bunny", label: "Bunny", file: "models/bunny.obj" },
  { id: "teapot", label: "Teapot", file: "models/teapot.obj" },
  { id: "elephant", label: "Elephant", file: "models/elephant.obj" },
  { id: "dragon", label: "Dragon", file: "models/dragon.obj" },
  { id: "chess-king", label: "Chess King", file: "models/ChessKing.obj" },
  { id: "sponza", label: "Sponza", file: "models/sponza.obj" }
];

export type VoxelParams = {
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

export const parseObjFallback = (input: string) => {
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

export const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

export const asNumber = (value: unknown, fallback: number) => {
  const num = Number(value);
  return Number.isFinite(num) ? num : fallback;
};

export const asInt = (value: unknown, fallback: number) =>
  Math.floor(asNumber(value, fallback));

export const asBool = (value: unknown, fallback: boolean) =>
  typeof value === "boolean" ? value : fallback;

export const normalizeRenderMode = (value: unknown) =>
  value === "cubes" || value === "points" ? value : "points";

export const normalizeFloat32Array = (input: Float32Array<ArrayBufferLike>) =>
  input.buffer instanceof ArrayBuffer ? input : new Float32Array(input);

export const computeAutoRenderChunk = (limits: GPUDevice["limits"] | null) => {
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

export const appendBrickBoundsLines = (
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
    [0, 1], [1, 2], [2, 3], [3, 0],
    [4, 5], [5, 6], [6, 7], [7, 4],
    [0, 4], [1, 5], [2, 6], [3, 7]
  ];
  for (const [a, b] of edges) {
    lines.push(corners[a][0], corners[a][1], corners[a][2]);
    lines.push(corners[b][0], corners[b][1], corners[b][2]);
  }
};
