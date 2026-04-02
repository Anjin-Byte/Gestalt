import { voxelCenterWorld } from "./helpers";
import { pointInLattice } from "./latticeMask";
import type { LatticeParams, LatticeTopology } from "./types";

export const countCompactVoxels = (voxels: Int32Array): number => voxels.length / 4;

export const filterCompactVoxelsByLattice = (
  voxels: Int32Array,
  origin: [number, number, number],
  voxelSize: number,
  topology: LatticeTopology,
  lattice: LatticeParams
): Int32Array => {
  const kept: number[] = [];

  for (let i = 0; i < voxels.length; i += 4) {
    const vx = voxels[i];
    const vy = voxels[i + 1];
    const vz = voxels[i + 2];
    const material = voxels[i + 3];
    const center = voxelCenterWorld(vx, vy, vz, origin, voxelSize);

    if (pointInLattice(center, topology, lattice)) {
      kept.push(vx, vy, vz, material);
    }
  }

  return new Int32Array(kept);
};
