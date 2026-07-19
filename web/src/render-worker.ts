// The render worker's entry: composition only. Owns the OffscreenCanvas, its
// own WebGPU device, the engine, and the rAF loop — the main thread is a pure
// GUI shell forwarding input and routing scene blobs
// (docs/design/web-frontend-api.md §5, stage 7 phase 2). All dispatch logic
// lives in the testable core.
import init, { Engine } from "voxel-web";
import wasmUrl from "voxel-web/voxel_web_bg.wasm?url";

import { attachRenderWorker, type RenderWorkerScope } from "./render-worker-core";

let memory: WebAssembly.Memory | undefined;
attachRenderWorker(self as unknown as RenderWorkerScope, {
  createEngine: async (canvas, opts) => {
    const out = await init({ module_or_path: wasmUrl });
    memory = out.memory;
    return Engine.create_offscreen(canvas, opts);
  },
  raf: (cb) => {
    requestAnimationFrame(cb);
  },
  now: () => performance.now(),
  // The wasm heap gauge: `buffer.byteLength` is the grown-so-far size.
  heapBytes: () => memory?.buffer.byteLength ?? 0,
});
