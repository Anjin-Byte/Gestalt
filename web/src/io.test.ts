// IoClient protocol tests: the promise facade over the IO worker. The worker
// itself is a scripted fake installed as the global `Worker` — these tests
// own the reply schedule, so they can exercise orderings and failures the
// live worker rarely produces (Codex: adapter-layer logic is the TS layer's
// primary test responsibility).
import { MeshFormat } from "voxel-web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { IO_RECYCLE_HEAP_BYTES, IoClient } from "./io";
import { type JobProgress, type JobReply, type JobRequest } from "./io-protocol";

class FakeWorker {
  static instances: FakeWorker[] = [];
  onmessage: ((e: MessageEvent<JobReply>) => void) | null = null;
  onerror: ((e: ErrorEvent) => void) | null = null;
  readonly posted: { readonly message: JobRequest; readonly transfer: readonly unknown[] }[] =
    [];

  constructor(_url?: unknown, _opts?: unknown) {
    FakeWorker.instances.push(this);
  }

  postMessage(message: JobRequest, transfer?: unknown[]): void {
    this.posted.push({ message, transfer: transfer ?? [] });
  }

  terminated = false;
  terminate(): void {
    this.terminated = true;
  }
}

/** The worker the client under test constructed. */
function worker(): FakeWorker {
  const w = FakeWorker.instances.at(-1);
  if (!w) {
    throw new Error("no worker constructed");
  }
  return w;
}

function reply(r: JobReply): void {
  worker().onmessage?.({ data: r } as MessageEvent<JobReply>);
}

function fail(message: string): void {
  worker().onerror?.({ message } as ErrorEvent);
}

/** Drains microtasks so promise settlement (or its absence) is observable. */
function flush(): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

beforeEach(() => {
  FakeWorker.instances = [];
  vi.stubGlobal("Worker", FakeWorker);
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("IoClient request encoding", () => {
  it("posts voxelizeMesh with the exact job shape and moves the input buffer", () => {
    const client = new IoClient();
    const bytes = new Uint8Array([1, 2, 3]);
    const opts = { res: 128, truecolor: true, rotX: -90, gpuBake: false };
    void client.voxelizeMesh(bytes, MeshFormat.Glb, opts);
    expect(worker().posted).toHaveLength(1);
    const { message, transfer } = worker().posted[0] ?? { message: undefined, transfer: [] };
    expect(message).toEqual({
      id: 1,
      job: { kind: "voxelizeMesh", bytes, format: MeshFormat.Glb, opts },
    });
    expect(transfer).toHaveLength(1);
    expect(transfer[0]).toBe(bytes.buffer); // moved, not copied
  });

  it("posts buildFixture with no transferables", () => {
    const client = new IoClient();
    void client.buildFixture("caves", 2048);
    expect(worker().posted[0]?.message).toEqual({
      id: 1,
      job: { kind: "buildFixture", fixture: "caves", res: 2048 },
    });
    expect(worker().posted[0]?.transfer).toEqual([]);
  });

  it.each([
    ["decodeVox", (c: IoClient, b: Uint8Array) => c.decodeVox(b)] as const,
    ["decodeCvox", (c: IoClient, b: Uint8Array) => c.decodeCvox(b)] as const,
  ])("posts %s with the bytes moved", (kind, call) => {
    const client = new IoClient();
    const bytes = new Uint8Array([7]);
    void call(client, bytes);
    expect(worker().posted[0]?.message).toEqual({ id: 1, job: { kind, bytes } });
    expect(worker().posted[0]?.transfer[0]).toBe(bytes.buffer);
  });

  it.each([
    ["encodeVox", (c: IoClient, b: Uint8Array) => c.encodeVox(b)] as const,
    ["encodeCvox", (c: IoClient, b: Uint8Array) => c.encodeCvox(b)] as const,
  ])("posts %s with the scene blob moved", (kind, call) => {
    const client = new IoClient();
    const scene = new Uint8Array([9, 9]);
    void call(client, scene);
    expect(worker().posted[0]?.message).toEqual({ id: 1, job: { kind, scene } });
    expect(worker().posted[0]?.transfer[0]).toBe(scene.buffer);
  });

  it("assigns increasing ids across jobs", () => {
    const client = new IoClient();
    void client.buildFixture("a", 128);
    void client.buildFixture("b", 128);
    void client.decodeVox(new Uint8Array([1]));
    expect(worker().posted.map((p) => p.message.id)).toEqual([1, 2, 3]);
  });
});

describe("IoClient reply routing", () => {
  it("resolves the matching job with the replied bytes", async () => {
    const client = new IoClient();
    const p = client.buildFixture("dust", 128);
    const bytes = new Uint8Array([4, 5]);
    reply({ id: 1, ok: true, bytes, heapBytes: 0 });
    await expect(p).resolves.toBe(bytes);
  });

  it("rejects the matching job with the replied error message", async () => {
    const client = new IoClient();
    const p = client.buildFixture("nope", 128);
    reply({ id: 1, ok: false, error: "unknown fixture: nope", heapBytes: 0 });
    await expect(p).rejects.toThrow("unknown fixture: nope");
  });

  it("routes out-of-order replies by id, not arrival order", async () => {
    const client = new IoClient();
    const p1 = client.buildFixture("first", 128);
    const p2 = client.buildFixture("second", 128);
    const b2 = new Uint8Array([2]);
    const b1 = new Uint8Array([1]);
    reply({ id: 2, ok: true, bytes: b2, heapBytes: 0 });
    reply({ id: 1, ok: true, bytes: b1, heapBytes: 0 });
    await expect(p1).resolves.toBe(b1);
    await expect(p2).resolves.toBe(b2);
  });

  it("ignores replies for unknown ids", async () => {
    const client = new IoClient();
    const p = client.buildFixture("keep", 128);
    reply({ id: 99, ok: true, bytes: new Uint8Array(), heapBytes: 0 });
    reply({ id: 99, ok: false, error: "stray", heapBytes: 0 });
    await flush();
    // The real job is still pending and still resolvable.
    const bytes = new Uint8Array([1]);
    reply({ id: 1, ok: true, bytes, heapBytes: 0 });
    await expect(p).resolves.toBe(bytes);
  });

  it("delivers progress in order without settling the job", async () => {
    const client = new IoClient();
    const seen: JobProgress[] = [];
    let settled = false;
    const p = client
      .voxelizeMesh(new Uint8Array([1]), MeshFormat.Glb, {
        res: 128,
        truecolor: true,
        rotX: 0,
        gpuBake: true,
      }, (progress) => seen.push(progress))
      .finally(() => {
        settled = true;
      });
    reply({ id: 1, progress: { phase: "parse", done: 0, total: 0 } });
    reply({ id: 1, progress: { phase: "voxelize", done: 3, total: 9 } });
    await flush();
    expect(seen).toEqual([
      { phase: "parse", done: 0, total: 0 },
      { phase: "voxelize", done: 3, total: 9 },
    ]);
    expect(settled).toBe(false);
    const bytes = new Uint8Array([8]);
    reply({ id: 1, ok: true, bytes, heapBytes: 0 });
    await expect(p).resolves.toBe(bytes);
  });

  it("tolerates progress on a job that passed no onProgress", async () => {
    const client = new IoClient();
    const p = client.buildFixture("quiet", 128);
    reply({ id: 1, progress: { phase: "assemble", done: 1, total: 2 } });
    reply({ id: 1, ok: true, bytes: new Uint8Array(), heapBytes: 0 });
    await expect(p).resolves.toEqual(new Uint8Array());
  });

  it.each([
    ["buildFixture", (c: IoClient, cb: (p: JobProgress) => void) => c.buildFixture("caves", 2048, cb)],
    ["decodeVox", (c: IoClient, cb: (p: JobProgress) => void) => c.decodeVox(new Uint8Array([1]), cb)],
    ["decodeCvox", (c: IoClient, cb: (p: JobProgress) => void) => c.decodeCvox(new Uint8Array([1]), cb)],
    ["encodeVox", (c: IoClient, cb: (p: JobProgress) => void) => c.encodeVox(new Uint8Array([1]), cb)],
    ["encodeCvox", (c: IoClient, cb: (p: JobProgress) => void) => c.encodeCvox(new Uint8Array([1]), cb)],
  ] as const)("forwards progress to %s's onProgress before resolving", async (_name, call) => {
    const client = new IoClient();
    const seen: JobProgress[] = [];
    const p = call(client, (progress) => seen.push(progress));
    reply({ id: 1, progress: { phase: "generate", done: 0, total: 0 } });
    reply({ id: 1, progress: { phase: "assemble", done: 2, total: 4 } });
    const bytes = new Uint8Array([7]);
    reply({ id: 1, ok: true, bytes, heapBytes: 0 });
    await expect(p).resolves.toBe(bytes);
    expect(seen).toEqual([
      { phase: "generate", done: 0, total: 0 },
      { phase: "assemble", done: 2, total: 4 },
    ]);
  });

  it("ignores a duplicate completion for an already-settled id", async () => {
    const client = new IoClient();
    const p = client.buildFixture("once", 128);
    const bytes = new Uint8Array([1]);
    reply({ id: 1, ok: true, bytes, heapBytes: 0 });
    reply({ id: 1, ok: false, error: "late duplicate", heapBytes: 0 });
    await expect(p).resolves.toBe(bytes);
  });
});

describe("IoClient worker failure", () => {
  it("rejects every in-flight job with the script error", async () => {
    const client = new IoClient();
    const p1 = client.buildFixture("a", 128);
    const p2 = client.decodeVox(new Uint8Array([1]));
    fail("worker blew up");
    await expect(p1).rejects.toThrow("worker blew up");
    await expect(p2).rejects.toThrow("worker blew up");
  });

  it("falls back to a generic message when the event carries none", async () => {
    const client = new IoClient();
    const p = client.buildFixture("a", 128);
    fail("");
    await expect(p).rejects.toThrow("IO worker failed");
  });

  it("recycles the worker and stays usable after a script failure", async () => {
    const client = new IoClient();
    const doomed = client.buildFixture("a", 128);
    const first = worker();
    fail("boom");
    await expect(doomed).rejects.toThrow("boom");
    // The failed script may be dead: trusting it with the next job could hang
    // forever, so the client respawns.
    expect(first.terminated).toBe(true);
    expect(FakeWorker.instances).toHaveLength(2);
    const p = client.buildFixture("b", 128);
    expect(worker()).not.toBe(first);
    expect(worker().posted[0]?.message.id).toBe(2); // ids continue across workers
    const bytes = new Uint8Array([2]);
    reply({ id: 2, ok: true, bytes, heapBytes: 0 });
    await expect(p).resolves.toBe(bytes);
  });
});

describe("IoClient heap gauge and recycling", () => {
  it("reports every completion's heap gauge to onHeap", async () => {
    const client = new IoClient();
    const seen: number[] = [];
    client.onHeap((bytes) => seen.push(bytes));
    const p1 = client.buildFixture("a", 128);
    const p2 = client.buildFixture("b", 128);
    reply({ id: 1, ok: true, bytes: new Uint8Array(), heapBytes: 123 });
    reply({ id: 2, ok: false, error: "nope", heapBytes: 456 });
    await p1;
    await expect(p2).rejects.toThrow("nope");
    expect(seen).toEqual([123, 456]);
  });

  it("recycles an idle worker whose heap ratcheted past the threshold", async () => {
    const client = new IoClient();
    const seen: number[] = [];
    client.onHeap((bytes) => seen.push(bytes));
    const p = client.buildFixture("big", 512);
    const first = worker();
    reply({ id: 1, ok: true, bytes: new Uint8Array([1]), heapBytes: IO_RECYCLE_HEAP_BYTES + 1 });
    await p;
    expect(first.terminated).toBe(true);
    expect(FakeWorker.instances).toHaveLength(2);
    expect(seen).toEqual([IO_RECYCLE_HEAP_BYTES + 1, 0]); // the gauge, then the freed heap
    // The respawned worker serves the next job transparently.
    const p2 = client.buildFixture("next", 128);
    expect(worker()).not.toBe(first);
    expect(worker().posted[0]?.message.id).toBe(2);
    const bytes = new Uint8Array([2]);
    reply({ id: 2, ok: true, bytes, heapBytes: 0 });
    await expect(p2).resolves.toBe(bytes);
  });

  it("keeps the worker at or below the threshold", async () => {
    const client = new IoClient();
    const p = client.buildFixture("a", 128);
    reply({ id: 1, ok: true, bytes: new Uint8Array(), heapBytes: IO_RECYCLE_HEAP_BYTES });
    await p;
    expect(worker().terminated).toBe(false);
    expect(FakeWorker.instances).toHaveLength(1);
  });

  it("defers recycling while another job is still in flight", async () => {
    const client = new IoClient();
    const p1 = client.buildFixture("a", 128);
    const p2 = client.buildFixture("b", 128);
    reply({ id: 1, ok: true, bytes: new Uint8Array(), heapBytes: IO_RECYCLE_HEAP_BYTES + 1 });
    await p1;
    expect(FakeWorker.instances).toHaveLength(1); // job 2 pending: recycling would kill it
    reply({ id: 2, ok: true, bytes: new Uint8Array(), heapBytes: IO_RECYCLE_HEAP_BYTES + 1 });
    await p2;
    expect(FakeWorker.instances).toHaveLength(2); // idle now: recycled
  });
});
