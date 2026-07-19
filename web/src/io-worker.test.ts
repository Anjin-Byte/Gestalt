// io-worker-core tests: the serial job chain, per-job error isolation,
// progress forwarding, and the boot-failure path — driven with a scripted
// kernel and a recording scope, so the orderings (job B posted while job A is
// mid-GPU-await) are deterministic.
import { MeshFormat } from "voxel-web";
import { describe, expect, it, vi } from "vitest";

import { attachIoWorker, type IoKernelLike, type IoWorkerScope } from "./io-worker-core";
import { type JobReply, type JobRequest, type VoxelizeOptions } from "./io-protocol";

interface Sent {
  readonly message: JobReply;
  readonly transfer: readonly unknown[];
}

function makeScope(): { scope: IoWorkerScope; sent: Sent[] } {
  const sent: Sent[] = [];
  const scope: IoWorkerScope = {
    onmessage: null,
    postMessage(message, transfer) {
      sent.push({ message, transfer: transfer ?? [] });
    },
  };
  return { scope, sent };
}

function post(scope: IoWorkerScope, request: JobRequest): void {
  scope.onmessage?.({ data: request } as MessageEvent<JobRequest>);
}

interface Deferred<T> {
  readonly promise: Promise<T>;
  readonly resolve: (value: T) => void;
  readonly reject: (error: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function flush(): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

const OPTS: VoxelizeOptions = { res: 128, truecolor: true, rotX: 0, gpuBake: true };

/** The scripted wasm-heap gauge (rides every completion reply). */
const HEAP = 7_000_000;

/** A kernel whose every method is a spy with a configurable result. */
function makeKernel(overrides: Partial<IoKernelLike> = {}): IoKernelLike {
  return {
    voxelize_mesh: vi.fn(() => Promise.resolve(new Uint8Array([1]))),
    build_fixture: vi.fn(() => Promise.resolve(new Uint8Array([2]))),
    decode_vox: vi.fn(() => new Uint8Array([3])),
    decode_cvox: vi.fn(() => new Uint8Array([4])),
    encode_vox: vi.fn(() => new Uint8Array([5])),
    encode_cvox: vi.fn(() => new Uint8Array([6])),
    ...overrides,
  };
}

describe("attachIoWorker dispatch", () => {
  it("routes each job kind to its kernel method with exact arguments", async () => {
    const kernel = makeKernel();
    const { scope, sent } = makeScope();
    attachIoWorker(scope, Promise.resolve(kernel), () => HEAP);

    const mesh = new Uint8Array([10]);
    const vox = new Uint8Array([11]);
    const cvox = new Uint8Array([12]);
    const sceneA = new Uint8Array([13]);
    const sceneB = new Uint8Array([14]);
    post(scope, { id: 1, job: { kind: "voxelizeMesh", bytes: mesh, format: MeshFormat.Stl, opts: OPTS } });
    post(scope, { id: 2, job: { kind: "buildFixture", fixture: "perlin", res: 512 } });
    post(scope, { id: 3, job: { kind: "decodeVox", bytes: vox } });
    post(scope, { id: 4, job: { kind: "decodeCvox", bytes: cvox } });
    post(scope, { id: 5, job: { kind: "encodeVox", scene: sceneA } });
    post(scope, { id: 6, job: { kind: "encodeCvox", scene: sceneB } });
    await flush();

    // Every job forwards a progress callback (the last argument).
    expect(kernel.voxelize_mesh).toHaveBeenCalledWith(mesh, MeshFormat.Stl, OPTS, expect.any(Function));
    expect(kernel.build_fixture).toHaveBeenCalledWith("perlin", 512, expect.any(Function));
    expect(kernel.decode_vox).toHaveBeenCalledWith(vox, {}, expect.any(Function));
    expect(kernel.decode_cvox).toHaveBeenCalledWith(cvox, {}, expect.any(Function));
    expect(kernel.encode_vox).toHaveBeenCalledWith(sceneA, expect.any(Function));
    expect(kernel.encode_cvox).toHaveBeenCalledWith(sceneB, expect.any(Function));
    expect(sent.map((s) => s.message)).toEqual([
      { id: 1, ok: true, bytes: new Uint8Array([1]), heapBytes: HEAP },
      { id: 2, ok: true, bytes: new Uint8Array([2]), heapBytes: HEAP },
      { id: 3, ok: true, bytes: new Uint8Array([3]), heapBytes: HEAP },
      { id: 4, ok: true, bytes: new Uint8Array([4]), heapBytes: HEAP },
      { id: 5, ok: true, bytes: new Uint8Array([5]), heapBytes: HEAP },
      { id: 6, ok: true, bytes: new Uint8Array([6]), heapBytes: HEAP },
    ]);
  });

  it("transfers each result's buffer back to the main thread", async () => {
    const bytes = new Uint8Array([42]);
    const kernel = makeKernel({ build_fixture: vi.fn(() => Promise.resolve(bytes)) });
    const { scope, sent } = makeScope();
    attachIoWorker(scope, Promise.resolve(kernel), () => HEAP);
    post(scope, { id: 1, job: { kind: "buildFixture", fixture: "dust", res: 128 } });
    await flush();
    expect(sent[0]?.transfer).toEqual([bytes.buffer]);
  });
});

describe("attachIoWorker serial chain", () => {
  it("does not start job B while job A holds the kernel across an await", async () => {
    const gate = deferred<Uint8Array>();
    const kernel = makeKernel({ voxelize_mesh: vi.fn(() => gate.promise) });
    const { scope, sent } = makeScope();
    attachIoWorker(scope, Promise.resolve(kernel), () => HEAP);

    post(scope, { id: 1, job: { kind: "voxelizeMesh", bytes: new Uint8Array([1]), format: MeshFormat.Glb, opts: OPTS } });
    post(scope, { id: 2, job: { kind: "buildFixture", fixture: "dust", res: 128 } });
    await flush();
    expect(kernel.voxelize_mesh).toHaveBeenCalledTimes(1);
    expect(kernel.build_fixture).not.toHaveBeenCalled(); // strictly serial
    expect(sent).toEqual([]);

    gate.resolve(new Uint8Array([7]));
    await flush();
    expect(kernel.build_fixture).toHaveBeenCalledTimes(1);
    expect(sent.map((s) => s.message.id)).toEqual([1, 2]); // completion order = arrival order
  });

  it("keeps the chain alive after a failing job", async () => {
    const kernel = makeKernel({
      build_fixture: vi
        .fn<IoKernelLike["build_fixture"]>()
        .mockRejectedValueOnce(new Error("fixture exploded"))
        .mockResolvedValueOnce(new Uint8Array([2])),
    });
    const { scope, sent } = makeScope();
    attachIoWorker(scope, Promise.resolve(kernel), () => HEAP);
    post(scope, { id: 1, job: { kind: "buildFixture", fixture: "bad", res: 128 } });
    post(scope, { id: 2, job: { kind: "buildFixture", fixture: "good", res: 128 } });
    await flush();
    expect(sent.map((s) => s.message)).toEqual([
      { id: 1, ok: false, error: "fixture exploded", heapBytes: HEAP },
      { id: 2, ok: true, bytes: new Uint8Array([2]), heapBytes: HEAP },
    ]);
    expect(sent[0]?.transfer).toEqual([]); // error replies carry no buffer
  });

  it("stringifies non-Error rejections rather than replying undefined", async () => {
    const kernel = makeKernel({
      // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors -- the non-Error rejection is the case under test
      build_fixture: vi.fn(() => Promise.reject("raw string reason")),
    });
    const { scope, sent } = makeScope();
    attachIoWorker(scope, Promise.resolve(kernel), () => HEAP);
    post(scope, { id: 1, job: { kind: "buildFixture", fixture: "x", res: 128 } });
    await flush();
    expect(sent[0]?.message).toEqual({ id: 1, ok: false, error: "raw string reason", heapBytes: HEAP });
  });
});

describe("attachIoWorker progress forwarding", () => {
  it("posts one progress reply per kernel callback, then the result", async () => {
    const kernel = makeKernel({
      voxelize_mesh: vi.fn(
        (
          _bytes: Uint8Array,
          _format: MeshFormat,
          _opts: VoxelizeOptions,
          onProgress: (phase: string, done: number, total: number) => void,
        ) => {
          onProgress("voxelize", 3, 9);
          onProgress("colorBake", 0, 0);
          return Promise.resolve(new Uint8Array([1]));
        },
      ),
    });
    const { scope, sent } = makeScope();
    attachIoWorker(scope, Promise.resolve(kernel), () => HEAP);
    post(scope, { id: 5, job: { kind: "voxelizeMesh", bytes: new Uint8Array([1]), format: MeshFormat.Glb, opts: OPTS } });
    await flush();
    expect(sent.map((s) => s.message)).toEqual([
      { id: 5, progress: { phase: "voxelize", done: 3, total: 9 } },
      { id: 5, progress: { phase: "colorBake", done: 0, total: 0 } },
      { id: 5, ok: true, bytes: new Uint8Array([1]), heapBytes: HEAP },
    ]);
  });

  it("forwards progress for a non-mesh job too (encode's gather/write)", async () => {
    const kernel = makeKernel({
      encode_vox: vi.fn(
        (_scene: Uint8Array, onProgress: (phase: string, done: number, total: number) => void) => {
          onProgress("gather", 4, 4);
          onProgress("write", 0, 0);
          return new Uint8Array([9]);
        },
      ),
    });
    const { scope, sent } = makeScope();
    attachIoWorker(scope, Promise.resolve(kernel), () => HEAP);
    post(scope, { id: 2, job: { kind: "encodeVox", scene: new Uint8Array([1]) } });
    await flush();
    expect(sent.map((s) => s.message)).toEqual([
      { id: 2, progress: { phase: "gather", done: 4, total: 4 } },
      { id: 2, progress: { phase: "write", done: 0, total: 0 } },
      { id: 2, ok: true, bytes: new Uint8Array([9]), heapBytes: HEAP },
    ]);
  });
});

describe("attachIoWorker boot failure", () => {
  it("answers every job with the boot error instead of hanging", async () => {
    const { scope, sent } = makeScope();
    const ready = Promise.reject(new Error("no WebGPU adapter in worker"));
    attachIoWorker(scope, ready, () => HEAP);
    post(scope, { id: 1, job: { kind: "buildFixture", fixture: "a", res: 128 } });
    post(scope, { id: 2, job: { kind: "decodeVox", bytes: new Uint8Array([1]) } });
    await flush();
    expect(sent.map((s) => s.message)).toEqual([
      { id: 1, ok: false, error: "no WebGPU adapter in worker", heapBytes: HEAP },
      { id: 2, ok: false, error: "no WebGPU adapter in worker", heapBytes: HEAP },
    ]);
  });
});
