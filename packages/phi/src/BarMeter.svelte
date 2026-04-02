<script lang="ts">
  /**
   * BarMeter — Horizontal fill bar with threshold warning states.
   *
   * USE WHEN: Showing a value as a proportion of a known maximum — pool
   * usage, memory consumption, slot capacity, buffer fill level.
   *
   * PREFER INSTEAD:
   * - PropRow — when the value isn't a ratio (no meaningful "max")
   * - CounterRow — when the trend over time matters more than the current fill
   *
   * STATES: Normal (green) below threshold, warning (yellow) above, critical (red) at 90%+.
   * The threshold tick mark shows where the transition occurs.
   */
  let {
    label,
    value,
    max,
    valueLabel,
    threshold = 0.8,
    unit,
  }: {
    label: string;
    value: number;
    max: number;
    /** Override the right-side display text. Defaults to "value / max [unit]". */
    valueLabel?: string;
    /** Fill color transition point as a fraction 0–1. Default 0.8. */
    threshold?: number;
    unit?: string;
  } = $props();

  const pct = $derived(
    max > 0 ? Math.min(100, Math.max(0, (value / max) * 100)) : 0
  );

  const displayValue = $derived(
    valueLabel ??
      (unit
        ? `${value.toLocaleString()} / ${max.toLocaleString()} ${unit}`
        : `${value.toLocaleString()} / ${max.toLocaleString()}`)
  );

  const tier = $derived<"normal" | "warning" | "critical">(
    pct >= 90 ? "critical" :
    pct >= threshold * 100 ? "warning" :
    "normal"
  );
</script>

<div class="bar-meter">
  <div class="bar-header">
    <span class="bar-label">{label}</span>
    <span class="bar-value">{displayValue}</span>
  </div>
  <div class="bar-track">
    <div
      class="bar-fill"
      class:warn={tier === "warning"}
      class:crit={tier === "critical"}
      style="width: {pct}%"
    ></div>
    <div class="bar-threshold" style="left: {threshold * 100}%"></div>
  </div>
</div>

<style>
  /* ── Header ────────────────────────────────────────────────────────── */
  .bar-header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 5px;
  }

  .bar-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-subtle);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .bar-value {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-mid);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 140px;
    text-align: right;
  }

  /* ── Track ─────────────────────────────────────────────────────────── */
  /* overflow: visible lets the threshold tick extend above/below.        */
  .bar-track {
    position: relative;
    height: 4px;
    background: var(--fill-lo);
    border-radius: 2px;
    overflow: visible;
  }

  /* ── Fill ──────────────────────────────────────────────────────────── */
  /* clip-path keeps the fill rounded at its own right edge even though   */
  /* the parent has overflow: visible.                                     */
  .bar-fill {
    position: absolute;
    left: 0;
    top: 0;
    height: 100%;
    border-radius: 2px;
    background: oklch(0.80 0.16 250 / 55%);
    transition: width 0.15s ease, background 0.2s ease;
    min-width: 0;
  }

  .bar-fill.warn {
    background: oklch(0.76 0.12 80 / 65%);
  }

  .bar-fill.crit {
    background: oklch(0.68 0.18 25 / 70%);
  }

  /* ── Threshold tick ────────────────────────────────────────────────── */
  /* A thin vertical mark at the threshold fraction. Extends 2px above    */
  /* and below the 4px track so it reads at a glance without being loud.  */
  .bar-threshold {
    position: absolute;
    top: -2px;
    width: 1px;
    height: 8px;
    background: var(--stroke-strong);
    border-radius: 1px;
    pointer-events: none;
    /* Translate left by 50% so the tick is centered on the threshold.    */
    transform: translateX(-50%);
  }
</style>
