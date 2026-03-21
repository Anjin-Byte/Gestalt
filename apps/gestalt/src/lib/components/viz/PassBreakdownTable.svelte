<script lang="ts">
  import { PASS_COLORS, PASS_COLOR_FALLBACK } from "$lib/stores/timeline";
  import type { FrameSample } from "$lib/stores/timeline";

  let {
    history = [],
    budgetMs = 16.67,
  }: {
    history?: FrameSample[];
    /** Frame budget in ms used to color-shift the avg column. Default 16.67 (60fps). */
    budgetMs?: number;
  } = $props();

  interface PassRow {
    name: string;
    color: string;
    avg: number;
    stddev: number;
    /** Fraction of budget 0–1+. Drives avg text color. */
    budgetFraction: number;
  }

  const rows = $derived(computeRows(history));

  function computeRows(h: FrameSample[]): PassRow[] {
    if (h.length === 0) return [];

    const hasPasses = h.some(s => Object.keys(s.passes).length > 0);
    if (!hasPasses) {
      // CPU fallback — single total row
      const vals = h.map(s => s.totalMs);
      const avg = vals.reduce((a, b) => a + b, 0) / vals.length;
      const variance = vals.reduce((a, b) => a + b * b, 0) / vals.length - avg * avg;
      return [{
        name: "Frame total",
        color: PASS_COLOR_FALLBACK,
        avg,
        stddev: Math.sqrt(Math.max(0, variance)),
        budgetFraction: avg / budgetMs,
      }];
    }

    const sums: Record<string, number> = {};
    const sumSq: Record<string, number> = {};
    const counts: Record<string, number> = {};

    for (const sample of h) {
      for (const [pass, ms] of Object.entries(sample.passes)) {
        sums[pass]   = (sums[pass]   ?? 0) + ms;
        sumSq[pass]  = (sumSq[pass]  ?? 0) + ms * ms;
        counts[pass] = (counts[pass] ?? 0) + 1;
      }
    }

    return Object.keys(sums)
      .map(name => {
        const n = counts[name];
        const avg = sums[name] / n;
        const variance = sumSq[name] / n - avg * avg;
        return {
          name,
          color: PASS_COLORS[name] ?? PASS_COLOR_FALLBACK,
          avg,
          stddev: Math.sqrt(Math.max(0, variance)),
          budgetFraction: avg / budgetMs,
        };
      })
      .sort((a, b) => b.avg - a.avg);
  }

  /**
   * Interpolate avg text color from --text-mid (normal) toward --color-warning
   * (at budget) toward --color-destructive (over budget). Expressed as an inline
   * oklch string so the canvas-side OKLCH palette is consistent with DOM color.
   *
   * Below 60% of budget: neutral --text-mid oklch(0.88 0.004 250)
   * At 100% of budget:   warning oklch(0.76 0.12 80)
   * At 150%+ of budget:  destructive oklch(0.68 0.18 25)
   */
  function avgColor(fraction: number): string {
    if (fraction < 0.6) return "var(--text-mid)";
    if (fraction >= 1.5) return "var(--color-destructive)";
    if (fraction >= 1.0) {
      // interpolate warning → destructive over 1.0–1.5
      const t = Math.min((fraction - 1.0) / 0.5, 1);
      const L = 0.76 - t * 0.08;
      const C = 0.12 + t * 0.06;
      const H = 80  - t * 55;
      return `oklch(${L.toFixed(2)} ${C.toFixed(2)} ${H.toFixed(0)})`;
    }
    // interpolate neutral → warning over 0.6–1.0
    const t = (fraction - 0.6) / 0.4;
    const L = 0.88 - t * 0.12;
    const C = 0.004 + t * 0.116;
    const H = 250  - t * 170;
    return `oklch(${L.toFixed(2)} ${C.toFixed(2)} ${H.toFixed(0)})`;
  }
</script>

{#if rows.length > 0}
  <div class="pbt" role="table" aria-label="Pass timing breakdown">
    <div class="pbt-header" role="row">
      <span></span>
      <span class="pbt-hcell">Pass</span>
      <span class="pbt-hcell pbt-right">Avg</span>
      <span class="pbt-hcell pbt-right">±σ</span>
    </div>
    {#each rows as row}
      <div class="pbt-row" role="row">
        <span class="pbt-dot" style="background: {row.color}" role="presentation"></span>
        <span class="pbt-name" title={row.name}>{row.name}</span>
        <span class="pbt-avg" style="color: {avgColor(row.budgetFraction)}">{row.avg.toFixed(2)}</span>
        <span class="pbt-dev">±{row.stddev.toFixed(2)}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .pbt {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  /* grid: dot | name | avg | stddev */
  .pbt-header,
  .pbt-row {
    display: grid;
    grid-template-columns: 7px 1fr auto auto;
    align-items: center;
    column-gap: 6px;
  }

  /* ── Header ─────────────────────────────────────────────────────────── */

  .pbt-header {
    padding-bottom: 3px;
    border-bottom: 1px solid var(--stroke-lo);
    margin-bottom: 2px;
  }

  .pbt-hcell {
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 500;
    color: var(--text-faint);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }

  .pbt-right {
    text-align: right;
  }

  /* ── Data rows ───────────────────────────────────────────────────────── */

  .pbt-row {
    min-height: 16px;
  }

  .pbt-dot {
    width: 7px;
    height: 7px;
    border-radius: 1px;
    flex-shrink: 0;
  }

  .pbt-name {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-subtle);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pbt-avg {
    font-family: var(--font-mono);
    font-size: 10px;
    text-align: right;
    white-space: nowrap;
    /* color set via inline style */
    transition: color 0.3s ease;
  }

  .pbt-dev {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    text-align: right;
    white-space: nowrap;
  }
</style>
