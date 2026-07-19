// The render worker's brains — boot handshake, message dispatch, the frame
// loop with its stats cadence — separated from the boot side effects (wasm
// init, real WebGPU, real rAF) so tests can drive it with a scripted engine
// and a stepped clock. The entry script (render-worker.ts) is composition
// only.
import { type BrushTool, type CameraMode, type Falloff, type KeyAction } from "voxel-web";

import {
  FrameRing,
  takeScene,
  type RenderReply,
  type RenderRequest,
  type SceneInfoLike,
} from "./render-protocol";

// Structurally-typed worker global (this tsconfig serves the DOM app and
// cannot also load lib.webworker).
export interface RenderWorkerScope {
  onmessage: ((e: MessageEvent<RenderRequest>) => void) | null;
  postMessage(message: RenderReply, transfer?: Transferable[]): void;
}

/** The wasm `FrameStats` shape, structurally (the real class satisfies it). */
export interface FrameStatsLike {
  readonly frames: number;
  readonly nodes: number;
  readonly leaves: number;
  readonly voxels: number;
  readonly undo_depth: number;
  readonly redo_depth: number;
  readonly truecolor: boolean;
  free(): void;
}

/** What this worker needs from the wasm `Engine`, structurally — the real
 * class satisfies it; tests substitute fakes. */
export interface EngineLike {
  frame(dt_ms: number): void;
  stats(): FrameStatsLike;
  resize(width: number, height: number): void;
  key(action: KeyAction, down: boolean): void;
  pointer_delta(dx: number, dy: number): void;
  look_end(): void;
  pan(dx: number, dy: number): void;
  reset_pivot(): void;
  wheel(notches: number): void;
  set_camera_mode(mode: CameraMode): void;
  set_gtao(on: boolean): void;
  set_gtao_quality(preset: number): void;
  set_shadow_quality(quality: number): void;
  set_brush(
    tool: BrushTool,
    radius: number,
    strength: number,
    falloff: Falloff,
    color: number,
    invert: boolean,
  ): void;
  brush(x: number, y: number, pressure: number): void;
  brush_end(): void;
  hover(x: number, y: number): void;
  set_background(top: number, bottom: number): void;
  undo(): boolean;
  redo(): boolean;
  scene_info(): SceneInfoLike;
  install_scene(blob: Uint8Array, label: string, preserveCamera: boolean): SceneInfoLike;
  snapshot_scene(): Uint8Array;
}

/** The worker's real-world dependencies, injected by the entry script. */
export interface RenderWorkerDeps {
  /** Boots wasm + engine on the transferred canvas (the slow, real part). */
  createEngine(
    canvas: OffscreenCanvas,
    opts: { readonly res: number; readonly fixture: string },
  ): Promise<EngineLike>;
  /** `requestAnimationFrame`, injectable so tests can step frames. */
  raf(cb: (now: number) => void): void;
  /** `performance.now()`, injectable alongside `raf`. */
  now(): number;
  /** This worker's wasm heap size, attached to every stats message (the
   * memory-audit gauge — wasm heaps only grow). */
  heapBytes(): number;
}

/** How often aggregated stats go to the main-thread HUD, milliseconds. */
export const STATS_EVERY_MS = 250;

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Installs the message handler on `scope`: `init` boots the engine and
 * starts the frame loop; everything else dispatches to the live engine
 * (pre-ready messages other than `init` are dropped). */
export function attachRenderWorker(scope: RenderWorkerScope, deps: RenderWorkerDeps): void {
  let engine: EngineLike | undefined;
  let canvas: OffscreenCanvas | undefined;
  const ring = new FrameRing();

  async function boot(req: Extract<RenderRequest, { kind: "init" }>): Promise<void> {
    try {
      canvas = req.canvas;
      canvas.width = req.width;
      canvas.height = req.height;
      engine = await deps.createEngine(canvas, req.opts);
      scope.postMessage({ kind: "ready", ok: true, scene: takeScene(engine.scene_info()) });
      startLoop(engine);
    } catch (e) {
      scope.postMessage({ kind: "ready", ok: false, error: message(e) });
    }
  }

  function startLoop(engine: EngineLike): void {
    let last = deps.now();
    let lastStats = 0;
    const tick = (now: number): void => {
      engine.frame(now - last);
      ring.sample(now - last);
      last = now;
      if (now - lastStats >= STATS_EVERY_MS) {
        lastStats = now;
        const s = engine.stats();
        scope.postMessage({
          kind: "stats",
          stats: {
            ...ring.stats({
              frames: s.frames,
              nodes: s.nodes,
              leaves: s.leaves,
              voxels: s.voxels,
              undoDepth: s.undo_depth,
              redoDepth: s.redo_depth,
              truecolor: s.truecolor,
            }),
            heapBytes: deps.heapBytes(),
          },
        });
        s.free();
      }
      deps.raf(tick);
    };
    deps.raf(tick);
  }

  scope.onmessage = (e: MessageEvent<RenderRequest>) => {
    const req = e.data;
    if (req.kind === "init") {
      void boot(req);
      return;
    }
    if (!engine) {
      return; // pre-ready messages other than init are dropped
    }
    switch (req.kind) {
      case "resize":
        // Main cannot touch the transferred canvas; the backing store resizes
        // here, then the engine reconfigures its swapchain and output texture.
        if (canvas) {
          canvas.width = req.width;
          canvas.height = req.height;
        }
        engine.resize(req.width, req.height);
        break;
      case "key":
        engine.key(req.action, req.down);
        break;
      case "pointer":
        engine.pointer_delta(req.dx, req.dy);
        break;
      case "lookEnd":
        engine.look_end();
        break;
      case "pan":
        engine.pan(req.dx, req.dy);
        break;
      case "resetPivot":
        engine.reset_pivot();
        break;
      case "wheel":
        engine.wheel(req.notches);
        break;
      case "cameraMode":
        engine.set_camera_mode(req.mode);
        break;
      case "setGtao":
        engine.set_gtao(req.on);
        break;
      case "setGtaoQuality":
        engine.set_gtao_quality(req.preset);
        break;
      case "setShadowQuality":
        engine.set_shadow_quality(req.quality);
        break;
      case "setBrush":
        engine.set_brush(req.tool, req.radius, req.strength, req.falloff, req.color, req.invert);
        break;
      case "brushEnd":
        engine.brush_end();
        break;
      case "hover":
        engine.hover(req.x, req.y);
        break;
      case "background":
        engine.set_background(req.top, req.bottom);
        break;
      case "undo":
        engine.undo();
        break;
      case "redo":
        engine.redo();
        break;
      case "brush":
        try {
          engine.brush(req.x, req.y, req.pressure);
        } catch (err) {
          // The shell gates the brush on SceneMeta.editable; anything residual
          // is a bug worth seeing, not worth crashing the loop over.
          console.warn("brush:", err);
        }
        break;
      case "installScene":
        try {
          const scene = takeScene(engine.install_scene(req.blob, req.label, req.preserveCamera));
          ring.reset();
          scope.postMessage({ kind: "scene", id: req.id, ok: true, scene });
        } catch (err) {
          scope.postMessage({ kind: "scene", id: req.id, ok: false, error: message(err) });
        }
        break;
      case "snapshotScene":
        try {
          const bytes = engine.snapshot_scene();
          scope.postMessage({ kind: "bytes", id: req.id, ok: true, bytes }, [
            bytes.buffer as ArrayBuffer,
          ]);
        } catch (err) {
          scope.postMessage({ kind: "bytes", id: req.id, ok: false, error: message(err) });
        }
        break;
    }
  };
}
