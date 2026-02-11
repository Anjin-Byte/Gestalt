import type { TestbedModule } from "./types";
import { createHelloTriangleModule } from "./helloTriangle";
import { createWasmObjLoaderModule } from "./wasmObjLoader";
import { createWasmWebgpuDemoModule } from "./wasmWebgpuDemo";
import { createWasmVoxelizerModule } from "./wasmVoxelizer";
import { createWasmGreedyMesherModule } from "./wasmGreedyMesher";

export const createDefaultModules = (): TestbedModule[] => [
  createWasmGreedyMesherModule(),
  createWasmVoxelizerModule(),
  createHelloTriangleModule(),
  createWasmObjLoaderModule(),
  createWasmWebgpuDemoModule()
];
