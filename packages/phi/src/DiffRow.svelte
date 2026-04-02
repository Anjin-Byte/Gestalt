<script lang="ts">
  /**
   * DiffRow — Before/after comparison with delta and trend arrow.
   *
   * USE WHEN: Comparing two related values — CPU reference vs GPU output,
   * previous frame vs current frame, old count vs new count. Shows the
   * direction and magnitude of change.
   *
   * PREFER INSTEAD:
   * - PropRow — when you just need to show a single value
   * - CounterRow — when you want a time-series sparkline, not a point comparison
   * - BarMeter — when the comparison is against a known maximum
   *
   * PROPS: `prev` and `current` are numbers. `invertWarning` flips which
   * direction is "bad" (e.g., FPS dropping is bad, memory increasing is bad).
   */
  import { TrendingUp, TrendingDown, Minus } from "lucide-svelte";

  let {
    label,
    prev,
    current,
    unit,
    decimals = 0,
    invertWarning = false,
  }: {
    label: string;
    prev: number;
    current: number;
    /** Optional unit suffix (e.g. "ms", "MB"). */
    unit?: string;
    /** Decimal places for display. Default 0. */
    decimals?: number;
    /** When true, a negative delta is warning (e.g. FPS dropped). Default false (positive delta = warning). */
    invertWarning?: boolean;
  } = $props();

  const delta = $derived(current - prev);

  const tier = $derived<"up" | "down" | "neutral">(
    delta > 0 ? "up" : delta < 0 ? "down" : "neutral"
  );

  const isWarning = $derived(
    invertWarning ? tier === "down" : tier === "up"
  );

  const fmt = (v: number) => v.toFixed(decimals);
  const prevDisplay = $derived(fmt(prev));
  const currentDisplay = $derived(fmt(current));
  const unitSuffix = $derived(unit ? ` ${unit}` : "");
  const deltaDisplay = $derived(
    delta === 0 ? "0" : `${delta > 0 ? "+" : ""}${fmt(delta)}`
  );
</script>

<div class="diff-row">
  <span class="diff-label">{label}</span>
  <span class="diff-prev">{prevDisplay}{unitSuffix}</span>
  <span class="diff-arrow">→</span>
  <span class="diff-current">{currentDisplay}{unitSuffix}</span>
  <span
    class="diff-delta"
    class:diff-warning={isWarning && tier !== "neutral"}
    class:diff-good={!isWarning && tier !== "neutral"}
    class:diff-neutral={tier === "neutral"}
  >
    {#if tier === "up"}
      <TrendingUp size={10} strokeWidth={2} />
    {:else if tier === "down"}
      <TrendingDown size={10} strokeWidth={2} />
    {:else}
      <Minus size={10} strokeWidth={2} />
    {/if}
    {deltaDisplay}
  </span>
</div>

<style>
  .diff-row {
    display: grid;
    grid-template-columns: 1fr auto 16px auto auto;
    align-items: center;
    gap: 0 6px;
    min-height: 22px;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
  }

  .diff-label {
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    color: var(--text-subtle, #999);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .diff-prev {
    text-align: right;
    color: var(--text-faint, #555);
    white-space: nowrap;
  }

  .diff-arrow {
    text-align: center;
    color: var(--text-faint, #555);
    font-size: 10px;
  }

  .diff-current {
    text-align: right;
    color: var(--text-mid, #ccc);
    white-space: nowrap;
  }

  .diff-delta {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    font-size: 10px;
    font-weight: 500;
    padding: 1px 5px;
    border-radius: 3px;
    white-space: nowrap;
    min-width: 42px;
  }

  .diff-warning {
    color: var(--color-warning, oklch(0.76 0.12 80));
    background: oklch(0.76 0.12 80 / 0.1);
  }

  .diff-good {
    color: var(--color-success, oklch(0.72 0.17 160));
    background: oklch(0.72 0.17 160 / 0.1);
  }

  .diff-neutral {
    color: var(--text-faint, #555);
  }
</style>
