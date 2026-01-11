import type { TestbedModule } from "./types";
import { createHelloTriangleModule } from "./exampleHelloTriangle";
import { createVoxelsDebugModule } from "./exampleVoxelsDebug";
import { createWasmExampleModule } from "./wasmExample";
import { createWasmPointsModule } from "./wasmPoints";
import { createWasmObjLoaderModule } from "./wasmObjLoader";
import { createWasmWebgpuDemoModule } from "./wasmWebgpuDemo";
import { createWasmVoxelizerModule } from "./wasmVoxelizer";

export const createDefaultModules = (): TestbedModule[] => [
  createWasmVoxelizerModule(),
  createHelloTriangleModule(),
  createVoxelsDebugModule(),
  createWasmExampleModule(),
  createWasmPointsModule(),
  createWasmObjLoaderModule(),
  createWasmWebgpuDemoModule()
];
