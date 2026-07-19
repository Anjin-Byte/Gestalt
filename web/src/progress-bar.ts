// The segmented build bar: one segment per phase of whatever job is running,
// each filling on the kernel's real counts — never a fabricated overall
// percentage (true phase weights vary wildly, so a unified bar would be
// calibrated guesswork). Every job (mesh import, fixture build, decode, encode)
// drives the same bar with its own phase list (io-protocol's `*_PHASES`); a
// phase whose work reports no count pulses as indeterminate. The state
// derivation is pure and vitest-covered; the DOM writer below is a thin shell.

import { type JobProgress, type ProgressPhase } from "./io-protocol";

/** One segment's visual state. `fraction: undefined` = indeterminate (pulse). */
export type SegmentState =
  | { readonly kind: "pending" }
  | { readonly kind: "active"; readonly fraction: number | undefined }
  | { readonly kind: "done" };

/** Derives every segment's state from the latest progress event. Phases before
 * the active one read as done — including phases the job legitimately skipped
 * (e.g. cutout/colorBake on an untextured mesh). */
export function segmentStates(
  phases: readonly ProgressPhase[],
  latest: JobProgress | undefined,
): SegmentState[] {
  const at = latest === undefined ? -1 : phases.indexOf(latest.phase);
  return phases.map((_, i) => {
    if (latest === undefined || at === -1 || i > at) {
      return { kind: "pending" };
    }
    if (i < at) {
      return { kind: "done" };
    }
    return {
      kind: "active",
      fraction: latest.total > 0 ? Math.min(1, latest.done / latest.total) : undefined,
    };
  });
}

/** How long the completed (all-segments-full) bar stays visible before
 * hiding. Without this, a job ends straight from a partially-filled state —
 * the final segment can never read as done (no later phase ever reports past
 * it), so the bar would vanish without ever looking finished. */
export const FINISH_LINGER_MS = 400;

/** The DOM writer: (re)builds one segment element per phase of the running job
 * inside `root`. `begin(phases)` chooses the segment set, so the same bar
 * serves every job. */
export class BuildBar {
  readonly #root: HTMLElement;
  #phases: readonly ProgressPhase[] = [];
  #fills: HTMLElement[] = [];
  #hideTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(root: HTMLElement) {
    this.#root = root;
  }

  /** Starts a run over `phases`: rebuilds the segments for this job and shows
   * the bar all-pending (no stale fills from the previous job's phase set,
   * and no pending hide from the previous job's finish linger). */
  begin(phases: readonly ProgressPhase[]): void {
    clearTimeout(this.#hideTimer);
    this.#phases = phases;
    this.#root.replaceChildren();
    this.#fills = phases.map((phase) => {
      const seg = document.createElement("span");
      seg.className = "seg";
      seg.title = phase;
      const fill = document.createElement("i");
      seg.appendChild(fill);
      this.#root.appendChild(seg);
      return fill;
    });
    this.update(undefined);
    this.#root.hidden = false;
  }

  update(latest: JobProgress | undefined): void {
    const states = segmentStates(this.#phases, latest);
    for (const [i, fill] of this.#fills.entries()) {
      const state = states[i] ?? { kind: "pending" };
      const seg = fill.parentElement;
      const indeterminate = state.kind === "active" && state.fraction === undefined;
      seg?.classList.toggle("active", state.kind === "active");
      seg?.classList.toggle("indeterminate", indeterminate);
      if (indeterminate) {
        // The stylesheet's indeterminate rule owns the width (a full-width
        // pulse); an inline width would override it and leave the segment
        // invisibly empty.
        fill.style.width = "";
      } else {
        const width = state.kind === "done" ? 1 : state.kind === "active" ? (state.fraction ?? 0) : 0;
        fill.style.width = `${(width * 100).toFixed(1)}%`;
      }
    }
  }

  /** The job succeeded: fill every segment, linger briefly so the completed
   * bar is actually seen, then hide. */
  finish(): void {
    for (const fill of this.#fills) {
      const seg = fill.parentElement;
      seg?.classList.remove("active", "indeterminate");
      fill.style.width = "100.0%";
    }
    clearTimeout(this.#hideTimer);
    this.#hideTimer = setTimeout(() => {
      this.#root.hidden = true;
    }, FINISH_LINGER_MS);
  }

  /** Hides immediately — the failure path (no fake completion display). */
  end(): void {
    clearTimeout(this.#hideTimer);
    this.#root.hidden = true;
  }
}
