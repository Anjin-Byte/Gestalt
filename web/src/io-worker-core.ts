// The IO worker's brains — job dispatch, the serial chain, progress
// forwarding — separated from the boot side effects (wasm init, real WebGPU)
// so tests can drive it with a scripted kernel (Codex: the TS layer owns
// adapter-logic evidence). The entry script (io-worker.ts) is composition
// only.
import { type MeshFormat } from "voxel-web";

import {
  type Job,
  type JobReply,
  type JobRequest,
  type ProgressPhase,
  type VoxelizeOptions,
} from "./io-protocol";

/** The kernel's `(phaseKey, done, total)` progress callback shape. */
type OnProgress = (phase: string, done: number, total: number) => void;

// The worker global, typed structurally: this tsconfig serves the DOM app and
// cannot also load lib.webworker (conflicting globals); the two members used
// here are stable across every module-worker runtime.
export interface IoWorkerScope {
  onmessage: ((e: MessageEvent<JobRequest>) => void) | null;
  postMessage(message: JobReply, transfer?: Transferable[]): void;
}

/** What this worker needs from the wasm kernel, structurally — the real
 * `IoKernel` satisfies it; tests substitute fakes. */
export interface IoKernelLike {
  voxelize_mesh(
    bytes: Uint8Array,
    format: MeshFormat,
    opts: VoxelizeOptions,
    on_progress: OnProgress,
  ): Promise<Uint8Array>;
  build_fixture(fixture: string, res: number, on_progress: OnProgress): Promise<Uint8Array>;
  decode_vox(bytes: Uint8Array, opts: object, on_progress: OnProgress): Uint8Array;
  decode_cvox(bytes: Uint8Array, opts: object, on_progress: OnProgress): Uint8Array;
  encode_vox(scene_blob: Uint8Array, on_progress: OnProgress): Uint8Array;
  encode_cvox(scene_blob: Uint8Array, on_progress: OnProgress): Uint8Array;
}

/** Installs the job pump on `scope`: strictly serial execution (the kernel is
 * one bindgen object and a job holds its borrow across GPU awaits — the chain
 * guarantees no overlap), per-job error replies, and progress forwarding.
 * `heapBytes` reads this worker's wasm heap size; it rides every completion
 * reply so the client can watch the high-water mark and recycle the worker. */
export function attachIoWorker(
  scope: IoWorkerScope,
  ready: Promise<IoKernelLike>,
  heapBytes: () => number,
): void {
  // A boot failure surfaces as each job's error reply; this handler only stops
  // it from also raising an unhandled-rejection while no job is in flight.
  void ready.catch(() => undefined);

  async function run(job: Job, id: number): Promise<Uint8Array> {
    const kernel = await ready;
    // Every job forwards its kernel progress the same way. The kernel invokes
    // this on the meters' schedule; posting from a busy worker still delivers
    // immediately, so the bar stays live even through a synchronous phase.
    const onProgress: OnProgress = (phase, done, total) => {
      scope.postMessage({ id, progress: { phase: phase as ProgressPhase, done, total } });
    };
    switch (job.kind) {
      case "voxelizeMesh":
        return kernel.voxelize_mesh(job.bytes, job.format, job.opts, onProgress);
      case "buildFixture":
        return kernel.build_fixture(job.fixture, job.res, onProgress);
      case "decodeVox":
        return kernel.decode_vox(job.bytes, {}, onProgress);
      case "decodeCvox":
        return kernel.decode_cvox(job.bytes, {}, onProgress);
      case "encodeVox":
        return kernel.encode_vox(job.scene, onProgress);
      case "encodeCvox":
        return kernel.encode_cvox(job.scene, onProgress);
    }
  }

  let chain: Promise<void> = Promise.resolve();
  scope.onmessage = (e: MessageEvent<JobRequest>) => {
    const { id, job } = e.data;
    chain = chain.then(async () => {
      try {
        const bytes = await run(job, id);
        // The kernel copies results out of wasm memory into a fresh buffer, so
        // it is safe (and zero-copy) to transfer it to the main thread.
        scope.postMessage({ id, ok: true, bytes, heapBytes: heapBytes() }, [
          bytes.buffer as ArrayBuffer,
        ]);
      } catch (err) {
        const error = err instanceof Error ? err.message : String(err);
        scope.postMessage({ id, ok: false, error, heapBytes: heapBytes() });
      }
    });
  };
}
