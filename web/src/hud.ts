// The DOM HUD: scene identity/counts on scene changes, aggregated frame
// statistics at the host's coarse cadence. Pure presentation — sampling lives
// in the render hosts (worker- or main-side), which deliver ready-made
// `HudStats`.

import { type HudStats, type SceneMeta } from "./render-protocol";

/** `5585909` → `5.59M`, `142078` → `142.1K` (the native viewer's format). */
function compactCount(n: number): string {
  if (n >= 1e6) {
    return `${(n / 1e6).toFixed(2)}M`;
  }
  if (n >= 1e3) {
    return `${(n / 1e3).toFixed(1)}K`;
  }
  return n.toString();
}

/** Bytes for the heap gauge: mebibytes below a gibibyte (`96M`), gibibytes
 * with two decimals above (`1.25G`) — the ratchet reads at a glance. */
function compactBytes(n: number): string {
  if (n >= 2 ** 30) {
    return `${(n / 2 ** 30).toFixed(2)}G`;
  }
  return `${Math.round(n / 2 ** 20)}M`;
}

/** The HUD's DOM handles, bound once at boot. */
export interface HudElements {
  readonly fps: HTMLElement;
  readonly frame: HTMLElement;
  readonly frames: HTMLElement;
  readonly nodes: HTMLElement;
  readonly leaves: HTMLElement;
  readonly voxels: HTMLElement;
  readonly res: HTMLElement;
  readonly heap: HTMLElement;
}

/** Writes scene and frame-rate cells; owns no timing of its own. */
export class Hud {
  readonly #el: HudElements;
  /** The wasm-heap gauges (memory audit): render context and IO worker.
   * `undefined` until the first report from each. */
  #renderHeap: number | undefined;
  #ioHeap: number | undefined;

  constructor(el: HudElements) {
    this.#el = el;
  }

  /** Rewrites the frame-rate cells from one aggregated sample. */
  setStats(stats: HudStats): void {
    this.#el.fps.textContent = stats.fps > 0 ? stats.fps.toFixed(0) : "—";
    this.#el.frame.textContent =
      stats.frameAvg > 0
        ? `${stats.frameAvg.toFixed(1)} (${stats.frameMin.toFixed(1)}–${stats.frameMax.toFixed(1)}) ms`
        : "—";
    this.#el.frames.textContent = compactCount(stats.frames);
    // Live during brush edits (the scene cells refresh only on scene change).
    this.#el.nodes.textContent = compactCount(stats.nodes);
    this.#el.leaves.textContent = compactCount(stats.leaves);
    this.#el.voxels.textContent = compactCount(stats.voxels);
    this.#renderHeap = stats.heapBytes;
    this.#writeHeap();
  }

  /** Records the IO worker's wasm-heap gauge (0 = the worker was recycled). */
  setIoHeap(bytes: number): void {
    this.#ioHeap = bytes;
    this.#writeHeap();
  }

  #writeHeap(): void {
    const part = (v: number | undefined): string =>
      v === undefined ? "—" : compactBytes(v);
    this.#el.heap.textContent = `r ${part(this.#renderHeap)} · io ${part(this.#ioHeap)}`;
  }

  /** Rewrites the scene cells (called on scene changes, not per frame). */
  setScene(scene: SceneMeta): void {
    this.#el.nodes.textContent = compactCount(scene.nodes);
    this.#el.leaves.textContent = compactCount(scene.leaves);
    this.#el.voxels.textContent = compactCount(scene.voxels);
    this.#el.res.textContent = `${scene.res}³`;
  }
}
