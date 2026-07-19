// render-worker-core tests: the boot handshake, pre-ready message dropping,
// engine dispatch, and the frame loop's stats cadence — with a scripted
// engine, a hand-stepped rAF queue, and a controlled clock, so frame timing
// is exact rather than sampled.
import { BrushTool, CameraMode, Falloff, KeyAction } from "voxel-web";
import { describe, expect, it, vi } from "vitest";

import {
  attachRenderWorker,
  STATS_EVERY_MS,
  type EngineLike,
  type RenderWorkerScope,
} from "./render-worker-core";
import { type RenderReply, type RenderRequest, type SceneInfoLike } from "./render-protocol";

interface Sent {
  readonly message: RenderReply;
  readonly transfer: readonly unknown[];
}

function makeScope(): { scope: RenderWorkerScope; sent: Sent[] } {
  const sent: Sent[] = [];
  const scope: RenderWorkerScope = {
    onmessage: null,
    postMessage(message, transfer) {
      sent.push({ message, transfer: transfer ?? [] });
    },
  };
  return { scope, sent };
}

function post(scope: RenderWorkerScope, request: RenderRequest): void {
  scope.onmessage?.({ data: request } as MessageEvent<RenderRequest>);
}

function flush(): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

function sceneInfo(label: string): SceneInfoLike & { freed: () => number } {
  const free = vi.fn();
  return {
    label,
    nodes: 1,
    leaves: 2,
    voxels: 3,
    res: 128,
    editable: true,
    truecolor: false,
    free,
    freed: () => free.mock.calls.length,
  };
}

function makeEngine(): EngineLike & { statsFreed: number[] } {
  const statsFreed: number[] = [];
  return {
    statsFreed,
    frame: vi.fn(),
    stats: vi.fn(() => {
      const free = vi.fn(() => statsFreed.push(1));
      return { frames: 10, nodes: 1, leaves: 2, voxels: 3, undo_depth: 4, redo_depth: 1, truecolor: false, free };
    }),
    resize: vi.fn(),
    key: vi.fn(),
    pointer_delta: vi.fn(),
    look_end: vi.fn(),
    pan: vi.fn(),
    reset_pivot: vi.fn(),
    wheel: vi.fn(),
    set_camera_mode: vi.fn(),
    set_gtao: vi.fn(),
    set_gtao_quality: vi.fn(),
    set_shadow_quality: vi.fn(),
    set_brush: vi.fn(),
    brush: vi.fn(),
    brush_end: vi.fn(),
    hover: vi.fn(),
    set_background: vi.fn(),
    undo: vi.fn(() => true),
    redo: vi.fn(() => false),
    scene_info: vi.fn(() => sceneInfo("boot-scene")),
    install_scene: vi.fn(() => sceneInfo("installed")),
    snapshot_scene: vi.fn(() => new Uint8Array([9])),
  };
}

/** A booted worker plus the levers to step its loop deterministically. */
async function boot(engine: EngineLike = makeEngine()): Promise<{
  scope: RenderWorkerScope;
  sent: Sent[];
  engine: EngineLike;
  step: (now: number) => void;
  canvas: { width: number; height: number };
  createEngine: ReturnType<typeof vi.fn>;
}> {
  const { scope, sent } = makeScope();
  const rafQueue: ((now: number) => void)[] = [];
  const createEngine = vi.fn(() => Promise.resolve(engine));
  attachRenderWorker(scope, {
    createEngine,
    raf: (cb) => rafQueue.push(cb),
    now: () => 1000, // the loop's epoch
    heapBytes: () => 7_000_000, // the scripted wasm-heap gauge
  });
  const canvas = { width: 0, height: 0 };
  post(scope, {
    kind: "init",
    canvas: canvas as unknown as OffscreenCanvas,
    width: 640,
    height: 480,
    opts: { res: 128, fixture: "wire-lattice" },
  });
  await flush();
  const step = (now: number): void => {
    const cb = rafQueue.shift();
    if (!cb) {
      throw new Error("no frame scheduled");
    }
    cb(now);
  };
  return { scope, sent, engine, step, canvas, createEngine };
}

describe("attachRenderWorker boot", () => {
  it("sizes the canvas, boots the engine, and replies ready with the scene", async () => {
    const { sent, engine, canvas, createEngine } = await boot();
    expect(canvas).toEqual({ width: 640, height: 480 });
    expect(createEngine).toHaveBeenCalledWith(canvas, { res: 128, fixture: "wire-lattice" });
    expect(sent[0]?.message).toEqual({
      kind: "ready",
      ok: true,
      scene: {
        label: "boot-scene",
        nodes: 1,
        leaves: 2,
        voxels: 3,
        res: 128,
        editable: true,
        truecolor: false,
      },
    });
    // The wasm SceneInfo must be freed after copying.
    const info = (engine.scene_info as ReturnType<typeof vi.fn>).mock.results[0]?.value as {
      freed: () => number;
    };
    expect(info.freed()).toBe(1);
  });

  it("replies ready:false when engine creation fails", async () => {
    const { scope, sent } = makeScope();
    attachRenderWorker(scope, {
      createEngine: () => Promise.reject(new Error("no adapter")),
      raf: () => undefined,
      now: () => 0,
      heapBytes: () => 0,
    });
    post(scope, {
      kind: "init",
      canvas: { width: 0, height: 0 } as unknown as OffscreenCanvas,
      width: 1,
      height: 1,
      opts: { res: 128, fixture: "dust" },
    });
    await flush();
    expect(sent.map((s) => s.message)).toEqual([
      { kind: "ready", ok: false, error: "no adapter" },
    ]);
  });

  it("drops every non-init message that arrives before the engine exists", async () => {
    const { scope, sent } = makeScope();
    const engine = makeEngine();
    attachRenderWorker(scope, {
      createEngine: () => Promise.resolve(engine),
      raf: () => undefined,
      now: () => 0,
      heapBytes: () => 0,
    });
    post(scope, { kind: "key", action: KeyAction.Forward, down: true });
    post(scope, { kind: "brush", x: 1, y: 1, pressure: 1 });
    post(scope, { kind: "snapshotScene", id: 1 });
    await flush();
    expect(sent).toEqual([]); // not even an error reply — dropped by design
    expect(engine.key).not.toHaveBeenCalled();
  });
});

describe("attachRenderWorker frame loop", () => {
  it("feeds the engine exact dt values and reschedules every frame", async () => {
    const { engine, step } = await boot();
    step(1016); // first frame: dt = 1016 - 1000 (the now() epoch)
    step(1033);
    expect(engine.frame).toHaveBeenNthCalledWith(1, 16);
    expect(engine.frame).toHaveBeenNthCalledWith(2, 17);
  });

  it("posts aggregated stats on the cadence, freeing each wasm stats object", async () => {
    const { sent, engine, step } = await boot();
    step(1016); // 1016 - 0 >= 250 → posts stats
    const statsMessages = sent.filter((s) => s.message.kind === "stats");
    expect(statsMessages).toHaveLength(1);
    expect(statsMessages[0]?.message).toEqual({
      kind: "stats",
      stats: {
        fps: 62.5, // one 16 ms sample
        frameAvg: 16,
        frameMin: 16,
        frameMax: 16,
        frames: 10,
        nodes: 1,
        leaves: 2,
        voxels: 3,
        undoDepth: 4, // FrameStats depths ride the same cadence
        redoDepth: 1,
        truecolor: false,
        heapBytes: 7_000_000, // the injected gauge rides every stats message
      },
    });
    step(1032); // within the 250 ms window → no new stats
    expect(sent.filter((s) => s.message.kind === "stats")).toHaveLength(1);
    step(1016 + STATS_EVERY_MS); // window elapsed → posts again
    expect(sent.filter((s) => s.message.kind === "stats")).toHaveLength(2);
    expect((engine as ReturnType<typeof makeEngine>).statsFreed).toHaveLength(2);
  });
});

describe("attachRenderWorker dispatch", () => {
  it("forwards the data-plane messages to the engine verbatim", async () => {
    const { scope, engine, canvas } = await boot();
    post(scope, { kind: "resize", width: 800, height: 600 });
    post(scope, { kind: "key", action: KeyAction.Boost, down: false });
    post(scope, { kind: "pointer", dx: 4, dy: -6 });
    post(scope, { kind: "lookEnd" });
    post(scope, { kind: "pan", dx: 9, dy: -1 });
    post(scope, { kind: "resetPivot" });
    post(scope, { kind: "wheel", notches: -3 });
    post(scope, { kind: "cameraMode", mode: CameraMode.Orbit });
    post(scope, { kind: "setBrush", tool: BrushTool.Paint, radius: 5, strength: 0.8, falloff: Falloff.Linear, color: 0xff112233, invert: true });
    post(scope, { kind: "brush", x: 5, y: 6, pressure: 0.5 });
    post(scope, { kind: "brushEnd" });
    post(scope, { kind: "hover", x: 7, y: 8 });
    post(scope, { kind: "background", top: 11, bottom: 22 });
    post(scope, { kind: "undo" });
    post(scope, { kind: "redo" });
    expect(canvas).toEqual({ width: 800, height: 600 }); // backing store resized here
    expect(engine.resize).toHaveBeenCalledWith(800, 600);
    expect(engine.key).toHaveBeenCalledWith(KeyAction.Boost, false);
    expect(engine.pointer_delta).toHaveBeenCalledWith(4, -6);
    expect(engine.look_end).toHaveBeenCalledTimes(1);
    expect(engine.pan).toHaveBeenCalledWith(9, -1);
    expect(engine.reset_pivot).toHaveBeenCalledTimes(1);
    expect(engine.wheel).toHaveBeenCalledWith(-3);
    expect(engine.set_camera_mode).toHaveBeenCalledWith(CameraMode.Orbit);
    expect(engine.set_brush).toHaveBeenCalledWith(BrushTool.Paint, 5, 0.8, Falloff.Linear, 0xff112233, true);
    expect(engine.brush).toHaveBeenCalledWith(5, 6, 0.5);
    expect(engine.brush_end).toHaveBeenCalledTimes(1);
    expect(engine.hover).toHaveBeenCalledWith(7, 8);
    expect(engine.set_background).toHaveBeenCalledWith(11, 22);
    expect(engine.undo).toHaveBeenCalledTimes(1);
    expect(engine.redo).toHaveBeenCalledTimes(1);
  });

  it("warns and keeps serving when a brush call throws", async () => {
    const engine = makeEngine();
    engine.brush = vi.fn(() => {
      throw new Error("residual brush on non-editable scene");
    });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const { scope } = await boot(engine);
    post(scope, { kind: "brush", x: 1, y: 2, pressure: 1 });
    expect(warn).toHaveBeenCalled();
    post(scope, { kind: "wheel", notches: 1 }); // the loop is still alive
    expect(engine.wheel).toHaveBeenCalledWith(1);
    warn.mockRestore();
  });

  it("replies to installScene with the copied scene and frees the wasm object", async () => {
    const { scope, sent, engine } = await boot();
    const blob = new Uint8Array([1, 2]);
    post(scope, { kind: "installScene", id: 7, blob, label: "tokyo.glb", preserveCamera: true });
    expect(engine.install_scene).toHaveBeenCalledWith(blob, "tokyo.glb", true);
    expect(sent.at(-1)?.message).toEqual({
      kind: "scene",
      id: 7,
      ok: true,
      scene: {
        label: "installed",
        nodes: 1,
        leaves: 2,
        voxels: 3,
        res: 128,
        editable: true,
        truecolor: false,
      },
    });
    const info = (engine.install_scene as ReturnType<typeof vi.fn>).mock.results[0]
      ?.value as { freed: () => number };
    expect(info.freed()).toBe(1);
  });

  it("replies scene:false when install throws, without killing the worker", async () => {
    const engine = makeEngine();
    engine.install_scene = vi.fn(() => {
      throw new Error("bad scene blob magic");
    });
    const { scope, sent } = await boot(engine);
    post(scope, { kind: "installScene", id: 3, blob: new Uint8Array([0]), label: "junk", preserveCamera: false });
    expect(sent.at(-1)?.message).toEqual({
      kind: "scene",
      id: 3,
      ok: false,
      error: "bad scene blob magic",
    });
    post(scope, { kind: "snapshotScene", id: 4 });
    expect(sent.at(-1)?.message).toMatchObject({ kind: "bytes", id: 4, ok: true });
  });

  it("transfers snapshot bytes back rather than copying", async () => {
    const bytes = new Uint8Array([5, 5, 5]);
    const engine = makeEngine();
    engine.snapshot_scene = vi.fn(() => bytes);
    const { scope, sent } = await boot(engine);
    post(scope, { kind: "snapshotScene", id: 2 });
    expect(sent.at(-1)?.message).toEqual({ kind: "bytes", id: 2, ok: true, bytes });
    expect(sent.at(-1)?.transfer).toEqual([bytes.buffer]);
  });

  it("replies bytes:false when snapshot throws", async () => {
    const engine = makeEngine();
    engine.snapshot_scene = vi.fn(() => {
      throw new Error("no scene retained");
    });
    const { scope, sent } = await boot(engine);
    post(scope, { kind: "snapshotScene", id: 8 });
    expect(sent.at(-1)?.message).toEqual({
      kind: "bytes",
      id: 8,
      ok: false,
      error: "no scene retained",
    });
  });
});
