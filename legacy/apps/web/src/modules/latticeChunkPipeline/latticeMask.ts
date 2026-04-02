import type { LatticeParams, LatticeTopology } from "./types";

const sub = (a: [number, number, number], b: [number, number, number]): [number, number, number] => [
  a[0] - b[0],
  a[1] - b[1],
  a[2] - b[2],
];

const dot = (a: [number, number, number], b: [number, number, number]) =>
  a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

const fract = (v: number) => v - Math.floor(v);

export const pointSegmentDistanceSquared = (
  p: [number, number, number],
  a: [number, number, number],
  b: [number, number, number]
): number => {
  const ab = sub(b, a);
  const ap = sub(p, a);
  const ab2 = dot(ab, ab);
  if (ab2 <= 1e-12) {
    return dot(ap, ap);
  }
  const t = Math.max(0, Math.min(1, dot(ap, ab) / ab2));
  const closest: [number, number, number] = [
    a[0] + ab[0] * t,
    a[1] + ab[1] * t,
    a[2] + ab[2] * t,
  ];
  const d = sub(p, closest);
  return dot(d, d);
};

export const worldToLatticeLocal = (
  pWorld: [number, number, number],
  params: LatticeParams
): { local: [number, number, number] } => {
  const origin = params.latticeOrigin;
  const cell = params.cellSize;
  const shifted: [number, number, number] = [
    (pWorld[0] - origin[0]) / cell,
    (pWorld[1] - origin[1]) / cell,
    (pWorld[2] - origin[2]) / cell,
  ];

  return {
    local: [fract(shifted[0]), fract(shifted[1]), fract(shifted[2])],
  };
};

export const pointInLattice = (
  pWorld: [number, number, number],
  topology: LatticeTopology,
  params: LatticeParams
): boolean => {
  const { local } = worldToLatticeLocal(pWorld, params);
  const radiusNorm = params.strutRadius / params.cellSize;
  const radius2 = radiusNorm * radiusNorm;

  for (const [aIdx, bIdx] of topology.edges) {
    const a = topology.nodes[aIdx];
    const b = topology.nodes[bIdx];
    if (pointSegmentDistanceSquared(local, a, b) <= radius2) {
      return true;
    }
  }

  return false;
};
