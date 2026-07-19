// The IO worker's entry: composition only. Boots its own wasm instance and
// its own WebGPU device (every scene build and codec job runs here, off the
// main thread — docs/design/web-frontend-api.md §5, stage 7) and hands the
// message pump to the testable core.
import init, { IoKernel } from "voxel-web";
import wasmUrl from "voxel-web/voxel_web_bg.wasm?url";

import { attachIoWorker, type IoWorkerScope } from "./io-worker-core";

// Kernel boot starts immediately; jobs await it. A boot failure (no WebGPU in
// workers on this browser) surfaces as each job's error reply.
let memory: WebAssembly.Memory | undefined;
attachIoWorker(
  self as unknown as IoWorkerScope,
  (async () => {
    const out = await init({ module_or_path: wasmUrl });
    memory = out.memory;
    return IoKernel.create();
  })(),
  // The wasm heap gauge: `buffer.byteLength` is the grown-so-far size.
  () => memory?.buffer.byteLength ?? 0,
);
