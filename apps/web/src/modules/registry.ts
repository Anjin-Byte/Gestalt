import type { TestbedModule } from "./types";
import { createHelloTriangleModule } from "./helloTriangle";
import { createWasmObjLoaderModule } from "./wasmObjLoader";
import { createWasmWebgpuDemoModule } from "./wasmWebgpuDemo";
import { createWasmVoxelizerModule } from "./wasmVoxelizer";
import { createWasmGreedyMesherModule } from "./wasmGreedyMesher";
import { createVoxelChunkPipelineModule } from "./voxelChunkPipeline";

export const createDefaultModules = (): TestbedModule[] => [
  createVoxelChunkPipelineModule(),
  createWasmGreedyMesherModule(),
  createWasmVoxelizerModule(),
  createHelloTriangleModule(),
  createWasmObjLoaderModule(),
  createWasmWebgpuDemoModule()
];
