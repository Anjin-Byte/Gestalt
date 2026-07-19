// Main-thread facade over the IO worker: a Promise API that hides the message
// protocol, so callers read like the old direct engine calls
// (docs/design/web-frontend-api.md §5, stage 7).
import { type MeshFormat } from "voxel-web";

import {
  type Job,
  type JobProgress,
  type JobReply,
  type JobRequest,
  type VoxelizeOptions,
} from "./io-protocol";

interface Pending {
  readonly resolve: (bytes: Uint8Array) => void;
  readonly reject: (error: Error) => void;
  readonly onProgress?: ((progress: JobProgress) => void) | undefined;
}

/** The job surface the shell programs against. `IoClient` implements it over
 * the real worker; tests substitute plain fakes (the class's private fields
 * make the class type itself nominally un-fakeable). */
export interface IoJobs {
  voxelizeMesh(
    bytes: Uint8Array,
    format: MeshFormat,
    opts: VoxelizeOptions,
    onProgress?: (progress: JobProgress) => void,
  ): Promise<Uint8Array>;
  buildFixture(
    fixture: string,
    res: number,
    onProgress?: (progress: JobProgress) => void,
  ): Promise<Uint8Array>;
  decodeVox(bytes: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array>;
  decodeCvox(bytes: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array>;
  encodeVox(scene: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array>;
  encodeCvox(scene: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array>;
  /** Registers the observer of the worker's wasm-heap gauge (HUD display);
   * reports 0 after a recycle. */
  onHeap(cb: (bytes: number) => void): void;
}

/** Heap size past which an idle IO worker is recycled. wasm memory never
 * shrinks — a worker that ever peaked at a big build holds that footprint for
 * the rest of the session. Terminating and respawning the worker is the only
 * way to hand the memory back; the next job pays a kernel re-boot (wasm init
 * + device request), trivial next to any job big enough to trip this.
 *
 * This is deliberately independent of the Rust side's per-build sanity gate
 * (`WEB_SCENE_BUDGET_BYTES` in `scene_transfer.rs`, ~wasm32's hard 4 GiB
 * ceiling with margin — a build that big is rare and would fail on any
 * browser). *This* threshold targets the much narrower, real problem: Safari
 * specifically reloads a tab under memory pressure well below that hard
 * ceiling (real-world reports as low as ~1.5 GiB *total* for the tab, shared
 * by both kernel instances plus JS/DOM/GPU overhead) — so even an
 * individually-safe build is worth reclaiming once it's *actually observed*
 * (not estimated) sitting idle above a few hundred MiB, keeping the sustained
 * footprint well clear of that ceiling across a long session of builds. */
export const IO_RECYCLE_HEAP_BYTES = 384 * 2 ** 20;

/** The IO worker's client. One instance per app; jobs queue serially. */
export class IoClient implements IoJobs {
  #worker!: Worker;
  readonly #pending = new Map<number, Pending>();
  #nextId = 1;
  #onHeap: ((bytes: number) => void) | undefined;

  constructor() {
    this.#spawn();
  }

  onHeap(cb: (bytes: number) => void): void {
    this.#onHeap = cb;
  }

  #spawn(): void {
    this.#worker = new Worker(new URL("./io-worker.ts", import.meta.url), {
      type: "module",
    });
    this.#worker.onmessage = (e: MessageEvent<JobReply>) => {
      const reply = e.data;
      const pending = this.#pending.get(reply.id);
      if (!pending) {
        return;
      }
      if ("progress" in reply) {
        pending.onProgress?.(reply.progress);
        return; // the job is still running
      }
      this.#pending.delete(reply.id);
      if (reply.ok) {
        pending.resolve(reply.bytes);
      } else {
        pending.reject(new Error(reply.error));
      }
      this.#watchHeap(reply.heapBytes);
    };
    // A script-level worker failure fails every in-flight job loudly rather
    // than hanging their promises — and the script may be dead, so the worker
    // is recycled rather than trusted with the next job.
    this.#worker.onerror = (e: ErrorEvent) => {
      const error = new Error(e.message || "IO worker failed");
      for (const pending of this.#pending.values()) {
        pending.reject(error);
      }
      this.#pending.clear();
      this.#recycle();
    };
  }

  /** Publishes the gauge; recycles the worker when it is idle and its wasm
   * heap has ratcheted past the threshold (heaps never shrink in place). */
  #watchHeap(heapBytes: number): void {
    this.#onHeap?.(heapBytes);
    if (this.#pending.size === 0 && heapBytes > IO_RECYCLE_HEAP_BYTES) {
      this.#recycle();
    }
  }

  #recycle(): void {
    this.#worker.terminate();
    this.#spawn();
    this.#onHeap?.(0); // the ratcheted heap is gone with the old worker
  }

  voxelizeMesh(
    bytes: Uint8Array,
    format: MeshFormat,
    opts: VoxelizeOptions,
    onProgress?: (progress: JobProgress) => void,
  ): Promise<Uint8Array> {
    return this.#submit({ kind: "voxelizeMesh", bytes, format, opts }, [bytes.buffer], onProgress);
  }

  buildFixture(
    fixture: string,
    res: number,
    onProgress?: (progress: JobProgress) => void,
  ): Promise<Uint8Array> {
    return this.#submit({ kind: "buildFixture", fixture, res }, [], onProgress);
  }

  decodeVox(bytes: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    return this.#submit({ kind: "decodeVox", bytes }, [bytes.buffer], onProgress);
  }

  decodeCvox(bytes: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    return this.#submit({ kind: "decodeCvox", bytes }, [bytes.buffer], onProgress);
  }

  encodeVox(scene: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    return this.#submit({ kind: "encodeVox", scene }, [scene.buffer], onProgress);
  }

  encodeCvox(scene: Uint8Array, onProgress?: (progress: JobProgress) => void): Promise<Uint8Array> {
    return this.#submit({ kind: "encodeCvox", scene }, [scene.buffer], onProgress);
  }

  #submit(
    job: Job,
    transfer: ArrayBufferLike[],
    onProgress?: (progress: JobProgress) => void,
  ): Promise<Uint8Array> {
    const id = this.#nextId;
    this.#nextId += 1;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject, onProgress });
      const request: JobRequest = { id, job };
      // Input buffers are moved, not copied: the caller hands ownership over.
      this.#worker.postMessage(request, transfer as Transferable[]);
    });
  }
}
