// The render-worker protocol plus the HUD-facing data shapes shared by both
// render hosts (docs/design/web-frontend-api.md §5, stage 7 phase 2). Internal
// to this app: both ends build from the same source.
import { type BrushTool, type CameraMode, type Falloff, type KeyAction } from "voxel-web";

/** Shared device-pixel-ratio cap: rendering above 2× buys nothing visible. */
export const MAX_DPR = 2;

/** A scene's identity and counts, as plain data the DOM keeps. */
export interface SceneMeta {
  readonly label: string;
  readonly nodes: number;
  readonly leaves: number;
  readonly voxels: number;
  readonly res: number;
  /** Whether the scene takes brush edits (every scene since Stage A3). */
  readonly editable: boolean;
  /** Whether the scene is truecolor — the shell gates the Paint tool on it
   * (palette scenes can't paint until the promotion path). */
  readonly truecolor: boolean;
}

/** The wasm `SceneInfo` shape, structurally: the real class satisfies it, and
 * worker-core tests can substitute plain objects (the class type itself is
 * only constructible by the kernel). */
export interface SceneInfoLike {
  readonly label: string;
  readonly nodes: number;
  readonly leaves: number;
  readonly voxels: number;
  readonly res: number;
  readonly editable: boolean;
  readonly truecolor: boolean;
  free(): void;
}

/** Copies a wasm `SceneInfo` into plain data and frees the wasm object. */
export function takeScene(info: SceneInfoLike): SceneMeta {
  const scene: SceneMeta = {
    label: info.label,
    nodes: info.nodes,
    leaves: info.leaves,
    voxels: info.voxels,
    res: info.res,
    editable: info.editable,
    truecolor: info.truecolor,
  };
  info.free();
  return scene;
}

/** The kernel-side counters carried alongside frame times (live during
 * edits). */
export interface KernelCounts {
  readonly frames: number;
  readonly nodes: number;
  readonly leaves: number;
  readonly voxels: number;
  /** Strokes available to undo (0 disables the HUD's undo button). */
  readonly undoDepth: number;
  /** Strokes available to redo (0 disables the HUD's redo button). */
  readonly redoDepth: number;
  /** Whether the scene carries per-voxel colour — flips mid-session when the
   * first Paint stroke promotes a palette scene (the shell confirms it in
   * the status line). */
  readonly truecolor: boolean;
}

/** Aggregated frame statistics for the HUD, produced at a coarse cadence. */
export interface HudStats extends KernelCounts {
  readonly fps: number;
  readonly frameAvg: number;
  readonly frameMin: number;
  readonly frameMax: number;
  /** The render context's `WebAssembly.Memory` size. wasm heaps only grow,
   * so this is the live high-water gauge of the memory audit. */
  readonly heapBytes: number;
}

/** Rolling frame-time window (preallocated — nothing allocates per frame). */
export class FrameRing {
  readonly #history = new Float32Array(120);
  #count = 0;
  #next = 0;

  sample(dtMs: number): void {
    this.#history[this.#next] = dtMs;
    this.#next = (this.#next + 1) % this.#history.length;
    this.#count = Math.min(this.#count + 1, this.#history.length);
  }

  reset(): void {
    this.#count = 0;
    this.#next = 0;
  }

  /** Aggregates the window; the kernel counters pass through. The heap gauge
   * is the render host's to add — it is not a frame-time statistic. */
  stats(kernel: KernelCounts): Omit<HudStats, "heapBytes"> {
    let min = Infinity;
    let max = -Infinity;
    let sum = 0;
    for (let i = 0; i < this.#count; i += 1) {
      const v = this.#history[i] ?? 0;
      min = Math.min(min, v);
      max = Math.max(max, v);
      sum += v;
    }
    const avg = this.#count > 0 ? sum / this.#count : 0;
    return {
      fps: avg > 0 ? 1000 / avg : 0,
      frameAvg: avg,
      frameMin: this.#count > 0 ? min : 0,
      frameMax: this.#count > 0 ? max : 0,
      ...kernel,
    };
  }
}

/** Messages main → render worker. */
export type RenderRequest =
  | {
      readonly kind: "init";
      readonly canvas: OffscreenCanvas;
      readonly width: number;
      readonly height: number;
      readonly opts: { readonly res: number; readonly fixture: string };
    }
  | { readonly kind: "resize"; readonly width: number; readonly height: number }
  | { readonly kind: "key"; readonly action: KeyAction; readonly down: boolean }
  | { readonly kind: "pointer"; readonly dx: number; readonly dy: number }
  | { readonly kind: "lookEnd" }
  | { readonly kind: "pan"; readonly dx: number; readonly dy: number }
  | { readonly kind: "resetPivot" }
  | { readonly kind: "wheel"; readonly notches: number }
  | { readonly kind: "cameraMode"; readonly mode: CameraMode }
  // Effects (GTAO lighting), settings-panel controls: the AO on/off toggle,
  // the AO quality preset (0 Low, 1 Medium, 2 High, 3 Ultra), and the
  // sun-shadow quality (0 off, 1 low/coarse, 2 high/exact).
  | { readonly kind: "setGtao"; readonly on: boolean }
  | { readonly kind: "setGtaoQuality"; readonly preset: number }
  | { readonly kind: "setShadowQuality"; readonly quality: number }
  | {
      readonly kind: "setBrush";
      readonly tool: BrushTool;
      readonly radius: number;
      readonly strength: number;
      readonly falloff: Falloff;
      readonly color: number;
      readonly invert: boolean;
    }
  | {
      readonly kind: "brush";
      readonly x: number;
      readonly y: number;
      readonly pressure: number;
    }
  | { readonly kind: "brushEnd" }
  | { readonly kind: "hover"; readonly x: number; readonly y: number }
  | { readonly kind: "background"; readonly top: number; readonly bottom: number }
  | { readonly kind: "undo" }
  | { readonly kind: "redo" }
  | {
      readonly kind: "installScene";
      readonly id: number;
      readonly blob: Uint8Array;
      readonly label: string;
      /** Keep the current camera (a scene re-derivation) vs. reset to the
       * framing orbit (a new load). The shell decides from scene provenance. */
      readonly preserveCamera: boolean;
    }
  | { readonly kind: "snapshotScene"; readonly id: number };

/** Messages render worker → main. */
export type RenderReply =
  | { readonly kind: "ready"; readonly ok: true; readonly scene: SceneMeta }
  | { readonly kind: "ready"; readonly ok: false; readonly error: string }
  | { readonly kind: "stats"; readonly stats: HudStats }
  | { readonly kind: "scene"; readonly id: number; readonly ok: true; readonly scene: SceneMeta }
  | { readonly kind: "scene"; readonly id: number; readonly ok: false; readonly error: string }
  | { readonly kind: "bytes"; readonly id: number; readonly ok: true; readonly bytes: Uint8Array }
  | { readonly kind: "bytes"; readonly id: number; readonly ok: false; readonly error: string };
