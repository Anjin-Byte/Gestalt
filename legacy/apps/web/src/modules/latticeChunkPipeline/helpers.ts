export const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

export const asNumber = (value: unknown, fallback: number) => {
  const num = Number(value);
  return Number.isFinite(num) ? num : fallback;
};

export const asInt = (value: unknown, fallback: number) =>
  Math.round(asNumber(value, fallback));

export const asBool = (value: unknown, fallback: boolean) =>
  typeof value === "boolean" ? value : fallback;

export const asString = (value: unknown, fallback: string) =>
  typeof value === "string" ? value : fallback;

export const computeBounds = (positions: Float32Array) => {
  let minX = Infinity, minY = Infinity, minZ = Infinity;
  let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;

  for (let i = 0; i < positions.length; i += 3) {
    const x = positions[i];
    const y = positions[i + 1];
    const z = positions[i + 2];
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (z < minZ) minZ = z;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
    if (z > maxZ) maxZ = z;
  }

  if (!Number.isFinite(minX)) {
    return null;
  }

  return {
    min: [minX, minY, minZ] as [number, number, number],
    max: [maxX, maxY, maxZ] as [number, number, number],
  };
};

export const normalizeMeshInPlace = (positions: Float32Array) => {
  const bounds = computeBounds(positions);
  if (!bounds) {
    return;
  }

  const cx = (bounds.min[0] + bounds.max[0]) * 0.5;
  const cy = (bounds.min[1] + bounds.max[1]) * 0.5;
  const cz = (bounds.min[2] + bounds.max[2]) * 0.5;
  const extent = Math.max(
    bounds.max[0] - bounds.min[0],
    bounds.max[1] - bounds.min[1],
    bounds.max[2] - bounds.min[2],
  ) || 1;
  const scale = 1.0 / extent;

  for (let i = 0; i < positions.length; i += 3) {
    positions[i] = (positions[i] - cx) * scale;
    positions[i + 1] = (positions[i + 1] - cy) * scale;
    positions[i + 2] = (positions[i + 2] - cz) * scale;
  }
};

export const computeGridOrigin = (
  positions: Float32Array,
  gridDim: number,
  voxelSize: number,
  fitBounds: boolean
) => {
  if (fitBounds) {
    const bounds = computeBounds(positions);
    if (bounds) {
      const size = [
        bounds.max[0] - bounds.min[0],
        bounds.max[1] - bounds.min[1],
        bounds.max[2] - bounds.min[2],
      ];
      const extent = Math.max(size[0], size[1], size[2]) || 1;
      const resolvedVoxelSize = extent / gridDim;
      const half = extent * 0.5;
      const center: [number, number, number] = [
        (bounds.min[0] + bounds.max[0]) * 0.5,
        (bounds.min[1] + bounds.max[1]) * 0.5,
        (bounds.min[2] + bounds.max[2]) * 0.5,
      ];
      return {
        origin: [center[0] - half, center[1] - half, center[2] - half] as [number, number, number],
        voxelSize: resolvedVoxelSize,
      };
    }
  }

  const half = gridDim * voxelSize * 0.5;
  return {
    origin: [-half, -half, -half] as [number, number, number],
    voxelSize,
  };
};

export const voxelCenterWorld = (
  vx: number,
  vy: number,
  vz: number,
  origin: [number, number, number],
  voxelSize: number
): [number, number, number] => [
  origin[0] + (vx + 0.5) * voxelSize,
  origin[1] + (vy + 0.5) * voxelSize,
  origin[2] + (vz + 0.5) * voxelSize,
];
