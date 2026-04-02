<script lang="ts">
  /**
   * CounterRow — Numeric value with inline sparkline history.
   *
   * USE WHEN: Displaying a value that changes every frame or on a regular
   * interval, and the trend over time is important. The sparkline shows
   * the last N values as a mini line chart.
   *
   * PREFER INSTEAD:
   * - PropRow — when the value is static or the trend doesn't matter
   * - DiffRow — when comparing two specific values (not a time series)
   * - BarMeter — when the value is a ratio with a known maximum
   *
   * PROPS: `history` drives the sparkline. `warn`/`danger` color the endpoint dot.
   */
  import Sparkline from "./Sparkline.svelte";

  let {
    label,
    value,
    history = [],
    warn,
    danger,
  }: {
    label: string;
    /** Formatted display value (e.g. "14,302"). */
    value: string;
    /** Raw numeric history for the sparkline. */
    history?: number[];
    /** Value at which sparkline endpoint turns warning yellow. */
    warn?: number;
    /** Value at which sparkline endpoint turns destructive red. */
    danger?: number;
  } = $props();
</script>

<div class="counter-row">
  <span class="cr-label">{label}</span>
  <div class="cr-right">
    <span class="cr-val">{value}</span>
    <div class="cr-spark">
      <Sparkline values={history} height={20} {warn} {danger} />
    </div>
  </div>
</div>

<style>
  .counter-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 24px;
  }

  .cr-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-subtle);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .cr-right {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    justify-content: flex-end;
    min-width: 0;
  }

  .cr-val {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-mid);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .cr-spark {
    width: 64px;
    flex-shrink: 0;
  }
</style>
