import { writable } from "svelte/store";

export const HISTORY_FRAMES = 240;

export interface FrameSample {
  totalMs: number;
  /** Pass name → duration in ms. Empty when `timestamp-query` feature is unavailable. */
  passes: Record<string, number>;
}

/** OKLCH colors for the documented render passes. Hues spread for easy distinction. */
export const PASS_COLORS: Record<string, string> = {
  "I-3 Summary Rebuild": "oklch(0.78 0.14 200)",
  "R-2 Depth Prepass":   "oklch(0.75 0.16 170)",
  "R-3 Hi-Z Pyramid":    "oklch(0.74 0.17 140)",
  "R-4a Chunk Cull":     "oklch(0.80 0.15 110)",
  "R-4b Meshlet Cull":   "oklch(0.83 0.13 85)",
  "R-5 Color Pass":      "oklch(0.76 0.17 260)",
  "R-6 Cascade Build":   "oklch(0.73 0.16 290)",
  "R-7 Cascade Merge":   "oklch(0.75 0.15 320)",
};

export const PASS_COLOR_FALLBACK = "oklch(0.60 0.08 250)";

function createTimeline() {
  const { subscribe, update } = writable<FrameSample[]>([]);

  return {
    subscribe,
    /** Push a completed frame sample into the circular buffer. Safe to call from async readback callbacks. */
    push(sample: FrameSample): void {
      update(history => {
        if (history.length >= HISTORY_FRAMES) {
          return [...history.slice(1), sample];
        }
        return [...history, sample];
      });
    },
    clear(): void {
      update(() => []);
    },
  };
}

export const frameTimeline = createTimeline();

/* ── Diagnostic counters ────────────────────────────────────────────────── */

/**
 * GPU-side atomic counters read back each frame via the DiagCounters buffer.
 * null means no readback has completed yet (panel shows "—").
 * Call `diagCounters.update()` from the same async readback path as frameTimeline.push().
 */
export interface DiagCounters {
  meshlets_culled: number;
  chunks_empty_skipped: number;
  version_mismatches: number;
  summary_rebuilds: number;
  mesh_rebuilds: number;
  cascade_ray_hits: number;
}

/** Per-frame history of DiagCounters snapshots, capped at HISTORY_FRAMES. */
function createDiagHistory() {
  const { subscribe, update } = writable<DiagCounters[]>([]);
  return {
    subscribe,
    push(counters: DiagCounters): void {
      update(h => h.length >= HISTORY_FRAMES ? [...h.slice(1), counters] : [...h, counters]);
    },
    clear(): void { update(() => []); },
  };
}

export const diagHistory = createDiagHistory();

function createDiagCounters() {
  const { subscribe, set } = writable<DiagCounters | null>(null);
  return {
    subscribe,
    update(counters: DiagCounters): void { set(counters); diagHistory.push(counters); },
    clear(): void { set(null); diagHistory.clear(); },
  };
}

export const diagCounters = createDiagCounters();
