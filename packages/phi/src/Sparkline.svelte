<script lang="ts">
  import { onMount, onDestroy } from "svelte";

  let {
    values = [],
    color = "oklch(0.80 0.16 250)",
    height = 24,
    warn,
    danger,
  }: {
    values?: number[];
    /** Line and fill color. Default: --interactive blue. */
    color?: string;
    /** Canvas height in CSS px. Default 24. */
    height?: number;
    /** Value at which the endpoint dot turns warning yellow. */
    warn?: number;
    /** Value at which the endpoint dot turns destructive red. */
    danger?: number;
  } = $props();

  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let wrapperEl = $state<HTMLElement | null>(null);
  let canvasW = $state(0);
  let rafId: number | undefined;
  let ro: ResizeObserver | undefined;

  function draw() {
    rafId = requestAnimationFrame(draw);
    const canvas = canvasEl;
    if (!canvas || canvasW <= 0) return;

    const dpr = window.devicePixelRatio ?? 1;
    const W = canvasW;
    const H = height;

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

    /* Background — inset surface, matches TimelineCanvas and --fill-inset */
    ctx.fillStyle = "oklch(0.14 0.015 250)";
    ctx.fillRect(0, 0, W, H);

    if (values.length < 2) {
      ctx.restore();
      return;
    }

    /* Coordinate helpers — PAD insets all four edges uniformly so the line,
       fill, and dot never touch the border and always share the same grid. */
    const maxVal = Math.max(1, ...values);
    const PAD = 3;
    const toX = (i: number) => PAD + i * ((W - PAD * 2) / (values.length - 1));
    const toY = (v: number) => H - PAD - (v / maxVal) * (H - PAD * 2);

    /* ── Filled area ───────────────────────────────────────────────────── */
    ctx.beginPath();
    ctx.moveTo(toX(0), H - PAD);
    for (let i = 0; i < values.length; i++) {
      ctx.lineTo(toX(i), toY(values[i]));
    }
    ctx.lineTo(toX(values.length - 1), H - PAD);
    ctx.closePath();
    ctx.globalAlpha = 0.18;
    ctx.fillStyle = color;
    ctx.fill();
    ctx.globalAlpha = 1; /* required — stroke and dot are drawn next; without this they render at 18% opacity */

    /* ── Line ──────────────────────────────────────────────────────────── */
    ctx.beginPath();
    for (let i = 0; i < values.length; i++) {
      const x = toX(i);
      const y = toY(values[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    ctx.stroke();

    /* ── Endpoint dot — sits exactly on the last line point ────────────── */
    const lastVal = values[values.length - 1];
    const dotX = toX(values.length - 1);
    const dotY = toY(lastVal);

    let dotColor = color;
    if (danger !== undefined && lastVal >= danger) {
      dotColor = "oklch(0.68 0.18 25)";   /* --color-destructive */
    } else if (warn !== undefined && lastVal >= warn) {
      dotColor = "oklch(0.76 0.12 80)";   /* --color-warning */
    }

    ctx.beginPath();
    ctx.arc(dotX, dotY, 2.5, 0, Math.PI * 2);
    ctx.fillStyle = dotColor;
    ctx.fill();

    ctx.restore();
  }

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

<div class="sl-wrap" bind:this={wrapperEl}>
  <canvas
    bind:this={canvasEl}
    class="sl-canvas"
    style="height: {height}px"
    aria-hidden="true"
  ></canvas>
</div>

<style>
  .sl-wrap {
    width: 100%;
  }

  .sl-canvas {
    display: block;
    width: 100%;
    border-radius: 2px;
    border: 1px solid var(--stroke-lo);
    box-shadow: var(--shadow-inset);
  }
</style>
