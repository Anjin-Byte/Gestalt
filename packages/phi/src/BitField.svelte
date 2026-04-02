<script lang="ts">
  /**
   * BitField — Row of labeled 1-bit flag pills.
   *
   * USE WHEN: Displaying multiple boolean flags compactly — chunk state bits
   * (is_empty, is_resident, stale_mesh), pipeline feature flags, GPU capability flags.
   * Each flag is a small pill: green (on), gray (off), or yellow (unknown/tri-state).
   *
   * PREFER INSTEAD:
   * - StatusIndicator — for a single boolean health status
   * - CheckboxRow — when the user should be able to toggle the flag
   * - PropRow — when the state is better expressed as text
   *
   * PROPS: `flags` is an array of { label, value, title? }. Short labels (1-3 chars) work best.
   *
   * Originally:
   * The label IS the indicator — background color encodes state.
   * Useful for GPU pipeline state flags, visibility flags, packed bitfields.
   */
  export interface BitFieldFlag {
    /** Short label (1–3 chars ideal, rendered inside the pill). */
    label: string;
    /** Current value. true = on, false = off, undefined = unknown/tri-state. */
    value: boolean | undefined;
    /** Tooltip shown on hover (e.g. full flag name). */
    title?: string;
  }

  let {
    label,
    flags,
  }: {
    /** Row label shown on the left. */
    label?: string;
    /** Array of flag definitions. */
    flags: BitFieldFlag[];
  } = $props();
</script>

<div class="bitfield">
  {#if label}
    <span class="bf-label">{label}</span>
  {/if}
  <div class="bf-flags">
    {#each flags as flag}
      <span
        class="bf-flag"
        class:bf-on={flag.value === true}
        class:bf-off={flag.value === false}
        class:bf-unknown={flag.value === undefined}
        title={flag.title ?? flag.label}
      >{flag.label}</span>
    {/each}
  </div>
</div>

<style>
  .bitfield {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 22px;
  }

  .bf-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-subtle, #999);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .bf-flags {
    display: flex;
    gap: 3px;
    flex-wrap: wrap;
  }

  /* ── Pill base ──────────────────────────────────────────────────────────── */
  .bf-flag {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-mono, monospace);
    font-size: 10px;
    font-weight: 600;
    line-height: 1;
    padding: 2px 4px;
    min-width: 24px;
    border-radius: 3px;
    white-space: nowrap;
    cursor: default;
    box-sizing: border-box;
    transition: background 0.12s ease, color 0.12s ease;
  }

  /* ── On — filled accent, light text ─────────────────────────────────────── */
  .bf-on {
    background: oklch(0.72 0.17 160 / 0.22);
    color: oklch(0.82 0.14 160);
    border: 1px solid oklch(0.72 0.17 160 / 0.25);
  }

  /* ── Off — recessed, dim text ───────────────────────────────────────────── */
  .bf-off {
    background: var(--fill-lo, oklch(1 0 0 / 0.03));
    color: var(--text-faint, oklch(0.46 0.01 250));
    border: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.06));
  }

  /* ── Unknown — warning tint, italic ─────────────────────────────────────── */
  .bf-unknown {
    background: oklch(0.76 0.12 80 / 0.12);
    color: oklch(0.80 0.10 80);
    border: 1px solid oklch(0.76 0.12 80 / 0.18);
    font-style: italic;
  }
</style>
