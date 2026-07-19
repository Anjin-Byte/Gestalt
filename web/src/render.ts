// The render host: where the engine lives. The worker host is the primary
// topology (main thread = pure GUI shell); the local host is the fallback for
// browsers without worker rendering, preserving the phase-1 behavior
// (docs/design/web-frontend-api.md §5, stage 7 phase 2).
import init, {
  Engine,
  type BrushTool,
  type CameraMode,
  type Falloff,
  type KeyAction,
} from "voxel-web";
import wasmUrl from "voxel-web/voxel_web_bg.wasm?url";

import {
  FrameRing,
  MAX_DPR,
  takeScene,
  type HudStats,
  type RenderReply,
  type RenderRequest,
  type SceneMeta,
} from "./render-protocol";

/** Scene-build options for the initial fixture. */
export interface EngineOptions {
  readonly res: number;
  readonly fixture: string;
}

/** The shell's handle on rendering, wherever it runs. Input methods are the
 * data-plane sink `attachInput` feeds. */
export interface RenderHost {
  /** "worker" (OffscreenCanvas) or "local" (main-thread fallback). */
  readonly mode: "worker" | "local";
  start(canvas: HTMLCanvasElement, opts: EngineOptions): Promise<SceneMeta>;
  /** Installs a built scene. `preserveCamera` keeps the current view (a scene
   * re-derivation) instead of resetting to the framing orbit (a new load). */
  installScene(blob: Uint8Array, label: string, preserveCamera: boolean): Promise<SceneMeta>;
  snapshotScene(): Promise<Uint8Array>;
  resize(width: number, height: number): void;
  key(action: KeyAction, down: boolean): void;
  pointerDelta(dx: number, dy: number): void;
  /** The look-drag released — in orbit, commits the fling momentum. */
  lookEnd(): void;
  /** Alt + left-drag: pans the orbit pivot in the camera plane (pixels). */
  pan(dx: number, dy: number): void;
  /** Double-click: recentres the orbit pivot on the model's own frame. */
  resetPivot(): void;
  wheel(notches: number): void;
  /** Selects the camera control scheme (the HUD mode buttons). */
  setCameraMode(mode: CameraMode): void;
  /** Sets the brush configuration (control plane — the HUD tool palette).
   * `invert` is the Alt-held tool arm (Inflate → deflate). */
  setBrush(
    tool: BrushTool,
    radius: number,
    strength: number,
    falloff: Falloff,
    color: number,
    invert: boolean,
  ): void;
  /** One pointer event of a brush stroke at device-pixel `(x, y)` with pen
   * `pressure` in `[0, 1]` (1.0 for a mouse); consecutive events interpolate. */
  brush(x: number, y: number, pressure: number): void;
  /** Pointer released: the next brush event starts a fresh stroke. */
  brushEnd(): void;
  /** The hover pick for the cursor ring (negative coords = pointer left). */
  hover(x: number, y: number): void;
  /** The themed sky endpoints (top, bottom — RGBA8, R low), from the live
   * CSS tokens; the render background follows the stylesheet. */
  setBackground(top: number, bottom: number): void;
  /** Undoes the most recent stroke (control plane — `Cmd+Z` / HUD button).
   * Depth for button enablement rides `HudStats.undoDepth`. */
  undo(): void;
  /** Re-applies the most recently undone stroke (`Shift+Cmd+Z` / HUD). */
  redo(): void;
  onStats(cb: (stats: HudStats) => void): void;
  /** Releases the host's resources (a failed worker start must not leave its
   * worker — and that worker's wasm instance — idling for the session). */
  dispose(): void;
}

interface PendingScene {
  readonly resolve: (scene: SceneMeta) => void;
  readonly reject: (error: Error) => void;
}
interface PendingBytes {
  readonly resolve: (bytes: Uint8Array) => void;
  readonly reject: (error: Error) => void;
}

/** Rendering in a dedicated worker: rAF, device, and engine all off-main. */
export class WorkerRenderHost implements RenderHost {
  readonly mode = "worker" as const;
  readonly #worker: Worker;
  readonly #scenes = new Map<number, PendingScene>();
  readonly #bytes = new Map<number, PendingBytes>();
  #nextId = 1;
  #stats: ((stats: HudStats) => void) | undefined;
  #ready: PendingScene | undefined;

  constructor() {
    this.#worker = new Worker(new URL("./render-worker.ts", import.meta.url), {
      type: "module",
    });
    this.#worker.onmessage = (e: MessageEvent<RenderReply>) => {
      this.#dispatch(e.data);
    };
    this.#worker.onerror = (e: ErrorEvent) => {
      const error = new Error(e.message || "render worker failed");
      this.#ready?.reject(error);
      this.#ready = undefined;
      for (const p of this.#scenes.values()) {
        p.reject(error);
      }
      for (const p of this.#bytes.values()) {
        p.reject(error);
      }
      this.#scenes.clear();
      this.#bytes.clear();
    };
  }

  start(canvas: HTMLCanvasElement, opts: EngineOptions): Promise<SceneMeta> {
    const dpr = Math.min(window.devicePixelRatio, MAX_DPR);
    const width = Math.max(1, Math.round(canvas.clientWidth * dpr));
    const height = Math.max(1, Math.round(canvas.clientHeight * dpr));
    // The one-way handoff: after this, only the worker may touch the backing
    // store — a failed start therefore needs a fresh canvas element to fall
    // back onto (main.ts owns that swap).
    const offscreen = canvas.transferControlToOffscreen();
    return new Promise((resolve, reject) => {
      this.#ready = { resolve, reject };
      this.#post({ kind: "init", canvas: offscreen, width, height, opts }, [offscreen]);
    });
  }

  installScene(blob: Uint8Array, label: string, preserveCamera: boolean): Promise<SceneMeta> {
    const id = this.#nextId;
    this.#nextId += 1;
    return new Promise((resolve, reject) => {
      this.#scenes.set(id, { resolve, reject });
      this.#post({ kind: "installScene", id, blob, label, preserveCamera }, [blob.buffer]);
    });
  }

  snapshotScene(): Promise<Uint8Array> {
    const id = this.#nextId;
    this.#nextId += 1;
    return new Promise((resolve, reject) => {
      this.#bytes.set(id, { resolve, reject });
      this.#post({ kind: "snapshotScene", id }, []);
    });
  }

  resize(width: number, height: number): void {
    this.#post({ kind: "resize", width, height }, []);
  }
  key(action: KeyAction, down: boolean): void {
    this.#post({ kind: "key", action, down }, []);
  }
  pointerDelta(dx: number, dy: number): void {
    this.#post({ kind: "pointer", dx, dy }, []);
  }
  lookEnd(): void {
    this.#post({ kind: "lookEnd" }, []);
  }
  pan(dx: number, dy: number): void {
    this.#post({ kind: "pan", dx, dy }, []);
  }
  resetPivot(): void {
    this.#post({ kind: "resetPivot" }, []);
  }
  wheel(notches: number): void {
    this.#post({ kind: "wheel", notches }, []);
  }
  setCameraMode(mode: CameraMode): void {
    this.#post({ kind: "cameraMode", mode }, []);
  }
  setBrush(
    tool: BrushTool,
    radius: number,
    strength: number,
    falloff: Falloff,
    color: number,
    invert: boolean,
  ): void {
    this.#post({ kind: "setBrush", tool, radius, strength, falloff, color, invert }, []);
  }
  brush(x: number, y: number, pressure: number): void {
    this.#post({ kind: "brush", x, y, pressure }, []);
  }
  brushEnd(): void {
    this.#post({ kind: "brushEnd" }, []);
  }
  hover(x: number, y: number): void {
    this.#post({ kind: "hover", x, y }, []);
  }
  setBackground(top: number, bottom: number): void {
    this.#post({ kind: "background", top, bottom }, []);
  }
  undo(): void {
    this.#post({ kind: "undo" }, []);
  }
  redo(): void {
    this.#post({ kind: "redo" }, []);
  }
  onStats(cb: (stats: HudStats) => void): void {
    this.#stats = cb;
  }

  dispose(): void {
    this.#worker.terminate();
  }

  #post(message: RenderRequest, transfer: (Transferable | ArrayBufferLike)[]): void {
    this.#worker.postMessage(message, transfer as Transferable[]);
  }

  #dispatch(reply: RenderReply): void {
    switch (reply.kind) {
      case "ready":
        if (reply.ok) {
          this.#ready?.resolve(reply.scene);
        } else {
          this.#ready?.reject(new Error(reply.error));
        }
        this.#ready = undefined;
        break;
      case "stats":
        this.#stats?.(reply.stats);
        break;
      case "scene": {
        const p = this.#scenes.get(reply.id);
        this.#scenes.delete(reply.id);
        if (reply.ok) {
          p?.resolve(reply.scene);
        } else {
          p?.reject(new Error(reply.error));
        }
        break;
      }
      case "bytes": {
        const p = this.#bytes.get(reply.id);
        this.#bytes.delete(reply.id);
        if (reply.ok) {
          p?.resolve(reply.bytes);
        } else {
          p?.reject(new Error(reply.error));
        }
        break;
      }
    }
  }
}

/** Main-thread fallback: the phase-1 topology (engine + rAF on main; the IO
 * worker still keeps builds off-thread). */
export class LocalRenderHost implements RenderHost {
  readonly mode = "local" as const;
  #engine: Engine | undefined;
  #canvas: HTMLCanvasElement | undefined;
  #memory: WebAssembly.Memory | undefined;
  readonly #ring = new FrameRing();
  #stats: ((stats: HudStats) => void) | undefined;

  async start(canvas: HTMLCanvasElement, opts: EngineOptions): Promise<SceneMeta> {
    const out = await init({ module_or_path: wasmUrl });
    this.#memory = out.memory;
    this.#canvas = canvas;
    const dpr = Math.min(window.devicePixelRatio, MAX_DPR);
    canvas.width = Math.max(1, Math.round(canvas.clientWidth * dpr));
    canvas.height = Math.max(1, Math.round(canvas.clientHeight * dpr));
    const engine = await Engine.create(canvas, opts);
    this.#engine = engine;

    let last = performance.now();
    let lastStats = 0;
    const tick = (now: number): void => {
      engine.frame(now - last);
      this.#ring.sample(now - last);
      last = now;
      if (now - lastStats >= 250) {
        lastStats = now;
        const s = engine.stats();
        this.#stats?.({
          ...this.#ring.stats({
            frames: s.frames,
            nodes: s.nodes,
            leaves: s.leaves,
            voxels: s.voxels,
            undoDepth: s.undo_depth,
            redoDepth: s.redo_depth,
            truecolor: s.truecolor,
          }),
          heapBytes: this.#memory?.buffer.byteLength ?? 0,
        });
        s.free();
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    return takeScene(engine.scene_info());
  }

  installScene(blob: Uint8Array, label: string, preserveCamera: boolean): Promise<SceneMeta> {
    const engine = this.#must();
    const scene = takeScene(engine.install_scene(blob, label, preserveCamera));
    this.#ring.reset();
    return Promise.resolve(scene);
  }

  snapshotScene(): Promise<Uint8Array> {
    return Promise.resolve(this.#must().snapshot_scene());
  }

  resize(width: number, height: number): void {
    if (this.#canvas) {
      this.#canvas.width = width;
      this.#canvas.height = height;
    }
    this.#engine?.resize(width, height);
  }
  key(action: KeyAction, down: boolean): void {
    this.#engine?.key(action, down);
  }
  pointerDelta(dx: number, dy: number): void {
    this.#engine?.pointer_delta(dx, dy);
  }
  lookEnd(): void {
    this.#engine?.look_end();
  }
  pan(dx: number, dy: number): void {
    this.#engine?.pan(dx, dy);
  }
  resetPivot(): void {
    this.#engine?.reset_pivot();
  }
  wheel(notches: number): void {
    this.#engine?.wheel(notches);
  }
  setCameraMode(mode: CameraMode): void {
    this.#engine?.set_camera_mode(mode);
  }
  setBrush(
    tool: BrushTool,
    radius: number,
    strength: number,
    falloff: Falloff,
    color: number,
    invert: boolean,
  ): void {
    this.#engine?.set_brush(tool, radius, strength, falloff, color, invert);
  }
  brush(x: number, y: number, pressure: number): void {
    try {
      this.#engine?.brush(x, y, pressure);
    } catch (err) {
      console.warn("brush:", err);
    }
  }
  brushEnd(): void {
    this.#engine?.brush_end();
  }
  hover(x: number, y: number): void {
    this.#engine?.hover(x, y);
  }
  setBackground(top: number, bottom: number): void {
    this.#engine?.set_background(top, bottom);
  }
  undo(): void {
    this.#engine?.undo();
  }
  redo(): void {
    this.#engine?.redo();
  }
  onStats(cb: (stats: HudStats) => void): void {
    this.#stats = cb;
  }

  dispose(): void {
    // The main-thread wasm instance cannot be unloaded; nothing to release.
  }

  #must(): Engine {
    if (!this.#engine) {
      throw new Error("render host not started");
    }
    return this.#engine;
  }
}
