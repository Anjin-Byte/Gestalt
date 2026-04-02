import type { TestbedModule } from "./types";
import { createHelloTriangleModule } from "./helloTriangle";
import { createWasmObjLoaderModule } from "./wasmObjLoader";
import { createWasmWebgpuDemoModule } from "./wasmWebgpuDemo";
import { createWasmVoxelizerModule } from "./wasmVoxelizer";
import { createWasmGreedyMesherModule } from "./wasmGreedyMesher";
import { createVoxelChunkPipelineModule } from "./voxelChunkPipeline";
import { createLatticeChunkPipelineModule } from "./latticeChunkPipeline";

export const createDefaultModules = (): TestbedModule[] => [
  createLatticeChunkPipelineModule(),
  createVoxelChunkPipelineModule(),
  createWasmGreedyMesherModule(),
  createWasmVoxelizerModule(),
  createHelloTriangleModule(),
  createWasmObjLoaderModule(),
  createWasmWebgpuDemoModule()
];
