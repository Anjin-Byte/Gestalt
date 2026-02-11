import type { MeshDescriptor, ModuleOutput, TestbedModule } from "../types";

const createTriangleMesh = (): MeshDescriptor => {
  const positions = new Float32Array([
    0, 1, 0,
    -1, -1, 0,
    1, -1, 0
  ]);

  const indices = new Uint32Array([0, 1, 2]);

  return { positions, indices };
};

export const createHelloTriangleModule = (): TestbedModule => ({
  id: "hello-triangle",
  name: "Hello Triangle",
  init: () => undefined,
  ui: (api) => {
    api.addSlider({
      id: "scale",
      label: "Scale",
      min: 0.5,
      max: 3,
      step: 0.1,
      initial: 1
    });
  },
  run: async (job) => {
    const mesh = createTriangleMesh();
    const scale = Number(job.params.scale ?? 1);
    const scaledPositions = new Float32Array(mesh.positions.length);
    for (let i = 0; i < mesh.positions.length; i += 1) {
      scaledPositions[i] = mesh.positions[i] * scale;
    }

    const output: ModuleOutput = {
      kind: "mesh",
      mesh: { ...mesh, positions: scaledPositions },
      label: "Cube"
    };

    return [output];
  }
});
