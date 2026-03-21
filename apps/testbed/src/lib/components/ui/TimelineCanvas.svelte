<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { HISTORY_FRAMES, PASS_COLORS, PASS_COLOR_FALLBACK } from "$lib/stores/timeline";
  import type { FrameSample } from "$lib/stores/timeline";

  export interface BudgetLine {
    ms: number;
    label: string;
    color?: string;
  }

  let {
    history = [],
    chartHeight = 80,
    maxMs = 50,
    showLegend = true,
    budgetLines = [],
    onHoverFrame,
  }: {
    history?: FrameSample[];
    /** CSS pixel height of the chart area. Default 80. */
    chartHeight?: number;
    /** Top of y-axis in ms. Default 50 — leaves breathing room above 33ms budget line. */
    maxMs?: number;
    /** Show the built-in DOM legend below the canvas. Set false when using PassBreakdownTable. */
    showLegend?: boolean;
    /** Additional budget reference lines beyond the built-in 16ms/33ms lines. */
    budgetLines?: BudgetLine[];
    /** Called with the hovered FrameSample, or null on mouse leave. */
    onHoverFrame?: (frame: FrameSample | null) => void;
  } = $props();

  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let wrapperEl = $state<HTMLElement | null>(null);
  let rafId: number | undefined;
  let ro: ResizeObserver | undefined;
  let canvasW = 0;
  let hoveredFrameIndex = $state<number | null>(null);

  /* ── Mouse interaction ──────────────────────────────────────────────── */

  function handleMouseMove(e: MouseEvent) {
    const canvas = canvasEl;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const xRatio = (e.clientX - rect.left) / rect.width;
    const snap = history;
    const colIndex = Math.floor(xRatio * HISTORY_FRAMES);
    const frameIndex = colIndex - (HISTORY_FRAMES - snap.length);
    if (frameIndex >= 0 && frameIndex < snap.length) {
      hoveredFrameIndex = frameIndex;
      onHoverFrame?.(snap[frameIndex]);
    } else {
      hoveredFrameIndex = null;
      onHoverFrame?.(null);
    }
  }

  function handleMouseLeave() {
    hoveredFrameIndex = null;
    onHoverFrame?.(null);
  }

  /* ── Legend (DOM, not canvas) ───────────────────────────────────────── */

  const legend = $derived(computeLegend(history));

  function computeLegend(
    h: FrameSample[]
  ): Array<{ name: string; avg: number; color: string }> {
    if (h.length === 0) return [];

    const hasPasses = h.some(s => Object.keys(s.passes).length > 0);
    if (!hasPasses) {
      const avg = h.reduce((sum, f) => sum + f.totalMs, 0) / h.length;
      return [{ name: "Frame total", avg, color: PASS_COLOR_FALLBACK }];
    }

    const sums: Record<string, number> = {};
    const counts: Record<string, number> = {};
    for (const sample of h) {
      for (const [pass, ms] of Object.entries(sample.passes)) {
        sums[pass] = (sums[pass] ?? 0) + ms;
        counts[pass] = (counts[pass] ?? 0) + 1;
      }
    }

    return Object.keys(sums)
      .map(name => ({
        name,
        avg: sums[name] / counts[name],
        color: PASS_COLORS[name] ?? PASS_COLOR_FALLBACK,
      }))
      .sort((a, b) => b.avg - a.avg);
  }

  /* ── Canvas draw loop ───────────────────────────────────────────────── */

  function draw() {
    rafId = requestAnimationFrame(draw);

    const canvas = canvasEl;
    if (!canvas || canvasW <= 0) return;

    const dpr = window.devicePixelRatio ?? 1;
    const W = canvasW;
    const H = chartHeight;

    const targetW = Math.round(W * dpr);
    const targetH = Math.round(H * dpr);
    if (canvas.width !== targetW || canvas.height !== targetH) {
      canvas.width = targetW;
      canvas.height = targetH;
    }

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.save();
    ctx.scale(dpr, dpr);

    /* Background — matches --fill-inset recessed tone */
    ctx.fillStyle = "oklch(0.14 0.015 250)";
    ctx.fillRect(0, 0, W, H);

    /* Budget lines */
    const msToY = (ms: number) => H - (ms / maxMs) * H;
    const y60 = msToY(16.67);
    const y30 = msToY(33.33);

    ctx.lineWidth = 1;
    ctx.setLineDash([2, 3]);

    ctx.strokeStyle = "oklch(0.68 0.18 25 / 40%)";
    ctx.beginPath(); ctx.moveTo(0, y30); ctx.lineTo(W, y30); ctx.stroke();

    ctx.strokeStyle = "oklch(0.76 0.12 80 / 40%)";
    ctx.beginPath(); ctx.moveTo(0, y60); ctx.lineTo(W, y60); ctx.stroke();

    ctx.setLineDash([]);

    /* Budget labels — right-aligned, above each line */
    ctx.font = "9px 'Geist Mono', ui-monospace, monospace";
    ctx.textBaseline = "bottom";
    ctx.textAlign = "right";
    ctx.fillStyle = "oklch(0.76 0.12 80 / 65%)";
    ctx.fillText("16ms", W - 2, y60 - 1);
    ctx.fillStyle = "oklch(0.68 0.18 25 / 65%)";
    ctx.fillText("33ms", W - 2, y30 - 1);

    /* Additional budget lines */
    for (const bl of budgetLines) {
      const yBl = msToY(bl.ms);
      const blColor = bl.color ?? "oklch(0.60 0.06 250 / 35%)";
      ctx.strokeStyle = blColor;
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 3]);
      ctx.beginPath(); ctx.moveTo(0, yBl); ctx.lineTo(W, yBl); ctx.stroke();
      ctx.setLineDash([]);
      ctx.font = "9px 'Geist Mono', ui-monospace, monospace";
      ctx.textBaseline = "bottom";
      ctx.textAlign = "right";
      ctx.fillStyle = bl.color ?? "oklch(0.60 0.06 250 / 55%)";
      ctx.fillText(bl.label, W - 2, yBl - 1);
    }

    /* Empty state */
    const snap = history;
    if (snap.length === 0) {
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.font = "10px 'Geist Mono', ui-monospace, monospace";
      ctx.fillStyle = "oklch(0.46 0.01 250)";
      ctx.fillText("awaiting GPU timestamps\u2026", W / 2, H / 2);
      ctx.restore();
      return;
    }

    /* Frame columns */
    const colW = W / HISTORY_FRAMES;

    for (let f = 0; f < snap.length; f++) {
      const sample = snap[f];
      const x = (HISTORY_FRAMES - snap.length + f) * colW;
      let y = H;

      const entries = Object.entries(sample.passes);

      if (entries.length === 0) {
        /* CPU fallback — single total bar */
        const barH = Math.min((sample.totalMs / maxMs) * H, H);
        ctx.fillStyle = "oklch(0.60 0.08 250 / 70%)";
        ctx.fillRect(x, H - barH, Math.max(colW - 0.5, 0.5), barH);
      } else {
        for (const [pass, ms] of entries) {
          const barH = (ms / maxMs) * H;
          ctx.fillStyle = PASS_COLORS[pass] ?? PASS_COLOR_FALLBACK;
          ctx.fillRect(x, y - barH, Math.max(colW - 0.5, 0.5), barH);
          y -= barH;
        }
      }

      /* Hover highlight */
      if (hoveredFrameIndex === f) {
        ctx.fillStyle = "oklch(1 0 0 / 10%)";
        ctx.fillRect(x, 0, Math.max(colW - 0.5, 0.5), H);
      }

      /* Budget-exceeded overlay — drawn on top of bars and hover highlight */
      if (sample.totalMs > 16.67) {
        ctx.fillStyle = "oklch(0.55 0.22 25 / 18%)";
        ctx.fillRect(x, 0, Math.max(colW - 0.5, 0.5), H);
      }
    }

    ctx.restore();
  }

  /* ── Lifecycle ──────────────────────────────────────────────────────── */

  onMount(() => {
    ro = new ResizeObserver(entries => {
      canvasW = entries[0]?.contentRect.width ?? 0;
    });
    if (wrapperEl) ro.observe(wrapperEl);
    rafId = requestAnimationFrame(draw);
  });

  onDestroy(() => {
    if (rafId !== undefined) cancelAnimationFrame(rafId);
    ro?.disconnect();
  });
</script>

<div class="tc-wrap" bind:this={wrapperEl}>
  <canvas
    bind:this={canvasEl}
    class="tc-canvas"
    class:tc-interactive={!!onHoverFrame}
    style="height: {chartHeight}px"
    aria-label="GPU frame timing waterfall"
    aria-hidden="true"
    onmousemove={handleMouseMove}
    onmouseleave={handleMouseLeave}
  ></canvas>

  {#if showLegend && legend.length > 0}
    <div class="tc-legend">
      {#each legend as entry}
        <div class="tc-legend-row">
          <span class="tc-dot" style="background: {entry.color}"></span>
          <span class="tc-name">{entry.name}</span>
          <span class="tc-val">{entry.avg.toFixed(2)}ms</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .tc-wrap {
    width: 100%;
  }

  .tc-canvas {
    display: block;
    width: 100%;
    border-radius: 3px;
    border: 1px solid var(--stroke-lo);
    box-shadow: var(--shadow-inset);
    /* Height is set via inline style from chartHeight prop. */
  }

  .tc-canvas.tc-interactive {
    cursor: crosshair;
  }

  /* ── Legend ──────────────────────────────────────────────────────────── */

  .tc-legend {
    margin-top: 5px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .tc-legend-row {
    display: flex;
    align-items: center;
    gap: 5px;
    min-height: 15px;
  }

  .tc-dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 1px;
  }

  .tc-name {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-subtle);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tc-val {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-mid);
    text-align: right;
    white-space: nowrap;
  }
</style>
