// @vitest-environment happy-dom
// WorkerRenderHost protocol tests: the canvas handoff, id-keyed pending maps,
// out-of-order reply routing, and script-failure rejection — over a scripted
// fake Worker, so every ordering is deterministic. (LocalRenderHost is the
// same interface over the real wasm engine; its logic lives in the kernel and
// is covered by the Rust suites — nothing to fake at this layer.)
import { BrushTool, CameraMode, Falloff, KeyAction } from "voxel-web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { WorkerRenderHost } from "./render";
import {
  type HudStats,
  type RenderReply,
  type RenderRequest,
  type SceneMeta,
} from "./render-protocol";

class FakeWorker {
  static instances: FakeWorker[] = [];
  onmessage: ((e: MessageEvent<RenderReply>) => void) | null = null;
  onerror: ((e: ErrorEvent) => void) | null = null;
  readonly posted: { readonly message: RenderRequest; readonly transfer: readonly unknown[] }[] =
    [];

  constructor(_url?: unknown, _opts?: unknown) {
    FakeWorker.instances.push(this);
  }

  postMessage(message: RenderRequest, transfer?: unknown[]): void {
    this.posted.push({ message, transfer: transfer ?? [] });
  }

  terminated = false;
  terminate(): void {
    this.terminated = true;
  }
}

function worker(): FakeWorker {
  const w = FakeWorker.instances.at(-1);
  if (!w) {
    throw new Error("no worker constructed");
  }
  return w;
}

function reply(r: RenderReply): void {
  worker().onmessage?.({ data: r } as MessageEvent<RenderReply>);
}

const SCENE: SceneMeta = {
  label: "wire-lattice",
  nodes: 10,
  leaves: 20,
  voxels: 30,
  res: 128,
  editable: true,
  truecolor: false,
};

const STATS: HudStats = {
  fps: 60,
  frameAvg: 16.6,
  frameMin: 15,
  frameMax: 18,
  frames: 1000,
  nodes: 10,
  leaves: 20,
  voxels: 30,
  undoDepth: 0,
  redoDepth: 0,
  truecolor: false,
  heapBytes: 7_000_000,
};

/** A canvas whose offscreen handoff yields a known token. */
function makeCanvas(token: object): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  Object.defineProperty(canvas, "transferControlToOffscreen", {
    value: () => token,
    configurable: true,
  });
  return canvas;
}

beforeEach(() => {
  FakeWorker.instances = [];
  vi.stubGlobal("Worker", FakeWorker);
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("WorkerRenderHost start", () => {
  it("posts init with the transferred offscreen canvas and measured size", async () => {
    const host = new WorkerRenderHost();
    const token = {};
    const canvas = makeCanvas(token);
    Object.defineProperty(canvas, "clientWidth", { value: 400 });
    Object.defineProperty(canvas, "clientHeight", { value: 300 });
    // A 3× display must clamp to the shared MAX_DPR of 2.
    Object.defineProperty(window, "devicePixelRatio", { value: 3, configurable: true });

    const started = host.start(canvas, { res: 128, fixture: "wire-lattice" });
    expect(worker().posted).toHaveLength(1);
    expect(worker().posted[0]?.message).toEqual({
      kind: "init",
      canvas: token,
      width: 800,
      height: 600,
      opts: { res: 128, fixture: "wire-lattice" },
    });
    expect(worker().posted[0]?.transfer).toEqual([token]);

    reply({ kind: "ready", ok: true, scene: SCENE });
    await expect(started).resolves.toEqual(SCENE);
  });

  it("clamps a zero-sized canvas to 1×1 rather than a dead swapchain", () => {
    const host = new WorkerRenderHost();
    void host.start(makeCanvas({}), { res: 128, fixture: "dust" }).catch(() => undefined);
    const init = worker().posted[0]?.message;
    expect(init).toMatchObject({ kind: "init", width: 1, height: 1 });
  });

  it("rejects start when the worker reports a failed boot", async () => {
    const host = new WorkerRenderHost();
    const started = host.start(makeCanvas({}), { res: 128, fixture: "dust" });
    reply({ kind: "ready", ok: false, error: "no WebGPU adapter" });
    await expect(started).rejects.toThrow("no WebGPU adapter");
  });
});

describe("WorkerRenderHost scene traffic", () => {
  it("moves the scene blob into installScene and routes the reply by id", async () => {
    const host = new WorkerRenderHost();
    const blob = new Uint8Array([1, 2, 3]);
    const p = host.installScene(blob, "tokyo.glb", false);
    expect(worker().posted[0]?.message).toEqual({
      kind: "installScene",
      id: 1,
      blob,
      label: "tokyo.glb",
      preserveCamera: false,
    });
    expect(worker().posted[0]?.transfer).toEqual([blob.buffer]);
    reply({ kind: "scene", id: 1, ok: true, scene: SCENE });
    await expect(p).resolves.toEqual(SCENE);
  });

  it("rejects installScene with the worker's error", async () => {
    const host = new WorkerRenderHost();
    const p = host.installScene(new Uint8Array([1]), "bad.vox", false);
    reply({ kind: "scene", id: 1, ok: false, error: "bad scene blob magic" });
    await expect(p).rejects.toThrow("bad scene blob magic");
  });

  it("resolves snapshotScene with the transferred bytes", async () => {
    const host = new WorkerRenderHost();
    const p = host.snapshotScene();
    expect(worker().posted[0]?.message).toEqual({ kind: "snapshotScene", id: 1 });
    const bytes = new Uint8Array([9, 9]);
    reply({ kind: "bytes", id: 1, ok: true, bytes });
    await expect(p).resolves.toBe(bytes);
  });

  it("routes interleaved scene and bytes replies by id across shared numbering", async () => {
    const host = new WorkerRenderHost();
    const install = host.installScene(new Uint8Array([1]), "a", false); // id 1
    const snapshot = host.snapshotScene(); // id 2
    const install2 = host.installScene(new Uint8Array([2]), "b", false); // id 3
    const bytes = new Uint8Array([7]);
    // Deliberately out of order.
    reply({ kind: "scene", id: 3, ok: true, scene: { ...SCENE, label: "b" } });
    reply({ kind: "bytes", id: 2, ok: true, bytes });
    reply({ kind: "scene", id: 1, ok: true, scene: { ...SCENE, label: "a" } });
    await expect(install).resolves.toMatchObject({ label: "a" });
    await expect(install2).resolves.toMatchObject({ label: "b" });
    await expect(snapshot).resolves.toBe(bytes);
  });

  it("ignores replies for ids it no longer tracks", async () => {
    const host = new WorkerRenderHost();
    const p = host.snapshotScene();
    reply({ kind: "bytes", id: 42, ok: false, error: "stray" });
    reply({ kind: "scene", id: 42, ok: true, scene: SCENE });
    const bytes = new Uint8Array([1]);
    reply({ kind: "bytes", id: 1, ok: true, bytes });
    await expect(p).resolves.toBe(bytes);
  });
});

describe("WorkerRenderHost data plane", () => {
  it("forwards each input setter as its exact message", () => {
    const host = new WorkerRenderHost();
    host.resize(640, 480);
    host.key(KeyAction.Forward, true);
    host.pointerDelta(3, -2);
    host.lookEnd();
    host.pan(5, 6);
    host.resetPivot();
    host.wheel(1.5);
    host.setCameraMode(CameraMode.Orbit);
    host.setBrush(BrushTool.Paint, 4, 0.5, Falloff.Sharp, 0xff00ff00, true);
    host.brush(10, 20, 0.75);
    host.brushEnd();
    host.hover(3, 4);
    host.setBackground(7, 9);
    host.undo();
    host.redo();
    expect(worker().posted.map((p) => p.message)).toEqual([
      { kind: "resize", width: 640, height: 480 },
      { kind: "key", action: KeyAction.Forward, down: true },
      { kind: "pointer", dx: 3, dy: -2 },
      { kind: "lookEnd" },
      { kind: "pan", dx: 5, dy: 6 },
      { kind: "resetPivot" },
      { kind: "wheel", notches: 1.5 },
      { kind: "cameraMode", mode: CameraMode.Orbit },
      { kind: "setBrush", tool: BrushTool.Paint, radius: 4, strength: 0.5, falloff: Falloff.Sharp, color: 0xff00ff00, invert: true },
      { kind: "brush", x: 10, y: 20, pressure: 0.75 },
      { kind: "brushEnd" },
      { kind: "hover", x: 3, y: 4 },
      { kind: "background", top: 7, bottom: 9 },
      { kind: "undo" },
      { kind: "redo" },
    ]);
    expect(worker().posted.every((p) => p.transfer.length === 0)).toBe(true);
  });

  it("delivers stats replies to the registered callback", () => {
    const host = new WorkerRenderHost();
    reply({ kind: "stats", stats: STATS }); // before registration: dropped, no crash
    const seen: HudStats[] = [];
    host.onStats((stats) => seen.push(stats));
    reply({ kind: "stats", stats: STATS });
    expect(seen).toEqual([STATS]);
  });
});

describe("WorkerRenderHost script failure", () => {
  it("rejects the pending start and every in-flight request", async () => {
    const host = new WorkerRenderHost();
    const started = host.start(makeCanvas({}), { res: 128, fixture: "dust" });
    const install = host.installScene(new Uint8Array([1]), "a", false);
    const snapshot = host.snapshotScene();
    worker().onerror?.({ message: "render worker crashed" } as ErrorEvent);
    await expect(started).rejects.toThrow("render worker crashed");
    await expect(install).rejects.toThrow("render worker crashed");
    await expect(snapshot).rejects.toThrow("render worker crashed");
  });

  it("falls back to a generic message when the event carries none", async () => {
    const host = new WorkerRenderHost();
    const p = host.snapshotScene();
    worker().onerror?.({ message: "" } as ErrorEvent);
    await expect(p).rejects.toThrow("render worker failed");
  });

  it("dispose terminates the worker (a failed start must not leak it)", () => {
    const host = new WorkerRenderHost();
    host.dispose();
    expect(worker().terminated).toBe(true);
  });
});
