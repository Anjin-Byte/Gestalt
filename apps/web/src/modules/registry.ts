import type { TestbedModule } from "./types";
import { createHelloTriangleModule } from "./exampleHelloTriangle";
import { createVoxelsDebugModule } from "./exampleVoxelsDebug";
import { createWasmExampleModule } from "./wasmExample";
import { createWasmPointsModule } from "./wasmPoints";
import { createWasmObjLoaderModule } from "./wasmObjLoader";
import { createWasmWebgpuDemoModule } from "./wasmWebgpuDemo";
import { createWasmVoxelizerModule } from "./wasmVoxelizer";
import { createWasmGreedyMesherModule } from "./wasmGreedyMesher";

export const createDefaultModules = (): TestbedModule[] => [
  createWasmGreedyMesherModule(),
  createWasmVoxelizerModule(),
  createHelloTriangleModule(),
  createVoxelsDebugModule(),
  createWasmExampleModule(),
  createWasmPointsModule(),
  createWasmObjLoaderModule(),
  createWasmWebgpuDemoModule()
];
