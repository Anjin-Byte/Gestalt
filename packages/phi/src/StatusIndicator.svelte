<script lang="ts">
  let {
    status = "idle",
    label,
    pulse,
  }: {
    status?: "ok" | "warning" | "error" | "idle";
    label?: string;
    pulse?: boolean;
  } = $props();

  const shouldPulse = $derived(pulse ?? status === "ok");
</script>

<span
  class="si-root"
  class:has-label={!!label}
>
  <span
    class="si-dot"
    class:si-ok={status === "ok"}
    class:si-warning={status === "warning"}
    class:si-error={status === "error"}
    class:si-idle={status === "idle"}
    class:si-pulse={shouldPulse}
    role="img"
    aria-label="Status: {label ?? status}"
  ></span>
  {#if label}
    <span class="si-label">{label}</span>
  {/if}
</span>

<style>
  .si-root {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  /* ── Dot ──────────────────────────────────────────────────────────── */
  .si-dot {
    position: relative;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;

    /* CSS variables for the glow, overridden per-status */
    --si-color: var(--stroke-strong);
    --si-glow:  oklch(1 0 0 / 0%);
    background: var(--si-color);
  }

  .si-dot.si-ok {
    --si-color: var(--color-success);
    --si-glow:  oklch(0.72 0.17 160 / 50%);
    box-shadow: 0 0 4px 0 var(--si-glow);
  }

  .si-dot.si-warning {
    --si-color: var(--color-warning);
    --si-glow:  oklch(0.76 0.12 80 / 40%);
    box-shadow: 0 0 4px 0 var(--si-glow);
  }

  .si-dot.si-error {
    --si-color: var(--color-destructive);
    --si-glow:  oklch(0.68 0.18 25 / 45%);
    box-shadow: 0 0 4px 0 var(--si-glow);
  }

  .si-dot.si-idle {
    opacity: 0.45;
  }

  /* ── Pulse glow via composited ::after ────────────────────────────── */
  /* The dot itself stays solid; ::after blurs and fades for the halo.   */
  /* opacity + transform are GPU-composited — no layout or paint cost.   */
  .si-dot::after {
    content: '';
    position: absolute;
    inset: -4px;
    border-radius: 50%;
    background: var(--si-color);
    filter: blur(4px);
    opacity: 0;
    transform: scale(0.6);
    pointer-events: none;
  }

  .si-dot.si-pulse::after {
    animation: si-pulse 10s ease-in-out infinite;
  }

  @keyframes si-pulse {
    0%, 100% { opacity: 0;    transform: scale(0.5); }
    55%       { opacity: 0.55; transform: scale(.6); }
  }

  /* ── Label ────────────────────────────────────────────────────────── */
  .si-label {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-subtle);
    white-space: nowrap;
  }
</style>
