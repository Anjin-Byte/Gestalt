import type { ModuleOutput, TestbedModule, VoxelsDescriptor } from "./types";

const createVoxelPositions = (size: number, spacing: number): Float32Array => {
  const positions: number[] = [];
  const offset = (size - 1) * spacing * 0.5;
  for (let x = 0; x < size; x += 1) {
    for (let y = 0; y < size; y += 1) {
      for (let z = 0; z < size; z += 1) {
        if ((x + y + z) % 2 === 0) {
          positions.push(x * spacing - offset, y * spacing - offset, z * spacing - offset);
        }
      }
    }
  }
  return new Float32Array(positions);
};

export const createVoxelsDebugModule = (): TestbedModule => ({
  id: "voxels-debug",
  name: "Voxels Debug",
  init: () => undefined,
  ui: (api) => {
    api.addSlider({
      id: "gridSize",
      label: "Grid Size",
      min: 2,
      max: 8,
      step: 1,
      initial: 4
    });
    api.addSlider({
      id: "spacing",
      label: "Spacing",
      min: 0.4,
      max: 1.2,
      step: 0.1,
      initial: 0.6
    });
  },
  run: async (job) => {
    const gridSize = Number(job.params.gridSize ?? 4);
    const spacing = Number(job.params.spacing ?? 0.6);
    const positions = createVoxelPositions(gridSize, spacing);
    const voxels: VoxelsDescriptor = {
      positions,
      voxelSize: spacing * 0.6,
      color: [0.45, 0.85, 0.95]
    };

    const output: ModuleOutput = {
      kind: "voxels",
      voxels,
      label: "Voxel Lattice"
    };

    return [output];
  }
});
