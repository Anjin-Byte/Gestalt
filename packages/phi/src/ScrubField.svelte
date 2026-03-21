<script lang="ts">
  import { Portal } from "bits-ui";
  import { ChevronLeft, ChevronRight } from "lucide-svelte";
  import { setHint, clearHint } from "$lib/stores/status";

  let {
    label,
    value,
    defaultValue,
    min,
    max,
    step = 0.01,
    decimals = 2,
    unit = "",
    onValueChange,
  }: {
    label: string;
    value: number;
    defaultValue?: number;
    min?: number;
    max?: number;
    step?: number;
    decimals?: number;
    unit?: string;
    onValueChange: (v: number) => void;
  } = $props();

  type Mode = "idle" | "scrubbing" | "editing";
  let mode = $state<Mode>("idle");
  let current = $state(value);
  let editText = $state("");
  let inputEl = $state<HTMLInputElement | undefined>(undefined);

  // Context menu
  let ctx = $state<{ open: boolean; x: number; y: number }>({ open: false, x: 0, y: 0 });

  // Drag tracking
  let dragStartX = 0;
  let dragStartValue = 0;
  let totalDragPx = 0;
  let trackRect: DOMRect | null = null;   // cached on pointerdown for spatial model
  const DRAG_THRESHOLD = 4;

  // Step-button repeat-on-hold — two-phase: immediate step, then delay, then repeat
  let stepTimeout: ReturnType<typeof setTimeout> | null = null;
  let stepInterval: ReturnType<typeof setInterval> | null = null;

  const displayValue = $derived(
    unit ? `${current.toFixed(decimals)} ${unit}` : current.toFixed(decimals)
  );

  const fillPct = $derived(
    min !== undefined && max !== undefined && max > min
      ? Math.max(0, Math.min(100, ((current - min) / (max - min)) * 100))
      : null
  );

  const atMin = $derived(min !== undefined && current <= min);
  const atMax = $derived(max !== undefined && current >= max);

  const hint = $derived(
    defaultValue !== undefined
      ? "[ − / + ] step  ·  drag to scrub  ·  click to type  ·  dbl-click to reset"
      : "[ − / + ] step  ·  drag to scrub  ·  click to type"
  );

  function clamp(v: number): number {
    let r = v;
    if (min !== undefined) r = Math.max(min, r);
    if (max !== undefined) r = Math.min(max, r);
    return r;
  }

  function applyValue(v: number) {
    const snapped = step !== 0 ? Math.round(v / step) * step : v;
    current = clamp(parseFloat(snapped.toFixed(decimals)));
    onValueChange(current);
  }

  // ── Step buttons ──────────────────────────────────────────────────────────

  const STEP_DELAY = 450;   // ms before repeat begins
  const STEP_RATE  = 80;    // ms between repeated steps

  function startStep(delta: number) {
    // Phase 1: one immediate step
    applyValue(current + delta);
    // Phase 2: after hold delay, start repeating
    stepTimeout = setTimeout(() => {
      stepTimeout = null;
      stepInterval = setInterval(() => applyValue(current + delta), STEP_RATE);
    }, STEP_DELAY);
  }

  function stopStep() {
    if (stepTimeout)  { clearTimeout(stepTimeout);   stepTimeout  = null; }
    if (stepInterval) { clearInterval(stepInterval); stepInterval = null; }
  }

  // ── Pointer scrubbing on track ────────────────────────────────────────────

  function onTrackPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    if (mode === "editing") return;
    e.preventDefault();
    const track = e.currentTarget as HTMLElement;
    track.setPointerCapture(e.pointerId);
    // Cache the track rect now — getBoundingClientRect during pointermove is
    // fine but caching here avoids repeated layout reads during fast drags.
    trackRect = track.getBoundingClientRect();
    dragStartX = e.clientX;
    dragStartValue = current;
    totalDragPx = 0;
    mode = "scrubbing";
    // No value change on pointerdown — we wait to see if this is a drag or a click.
  }

  function onTrackPointerMove(e: PointerEvent) {
    if (mode !== "scrubbing") return;
    const dx = e.clientX - dragStartX;
    totalDragPx = Math.abs(dx);

    if (min !== undefined && max !== undefined && trackRect) {
      // Spatial model: fill edge follows the cursor exactly.
      const frac = Math.max(0, Math.min(1, (e.clientX - trackRect.left) / trackRect.width));
      applyValue(min + frac * (max - min));
    } else {
      // Relative model (no bounds): delta × sensitivity.
      let sens = step || 0.01;
      if (e.shiftKey) sens *= 0.1;
      else if (e.ctrlKey || e.metaKey) sens *= 10;
      applyValue(dragStartValue + dx * sens);
    }
  }

  function onTrackPointerUp(_e: PointerEvent) {
    if (mode !== "scrubbing") return;
    if (totalDragPx < DRAG_THRESHOLD) enterEdit();
    else mode = "idle";
  }

  // ── Edit mode ─────────────────────────────────────────────────────────────

  function enterEdit() {
    mode = "editing";
    editText = current.toFixed(decimals);
    requestAnimationFrame(() => { inputEl?.focus(); inputEl?.select(); });
  }

  function commitEdit() {
    const v = parseFloat(editText);
    if (!Number.isNaN(v)) applyValue(v);
    mode = "idle";
  }

  function cancelEdit() { mode = "idle"; }

  function onInputKeyDown(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Enter") { e.preventDefault(); commitEdit(); }
    else if (e.key === "Escape") { e.preventDefault(); cancelEdit(); }
  }

  // ── Double-click reset ────────────────────────────────────────────────────

  function onTrackDblClick(e: MouseEvent) {
    e.preventDefault();
    if (defaultValue !== undefined) applyValue(defaultValue);
    mode = "idle";
  }

  // ── Context menu ──────────────────────────────────────────────────────────

  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    ctx = { open: true, x: e.clientX, y: e.clientY };
  }

  $effect(() => {
    if (!ctx.open) return;
    const close = () => { ctx = { ...ctx, open: false }; };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") close(); };
    const id = setTimeout(() => {
      document.addEventListener("click", close, true);
      document.addEventListener("contextmenu", close, true);
      document.addEventListener("keydown", onKey);
    }, 0);
    return () => {
      clearTimeout(id);
      document.removeEventListener("click", close, true);
      document.removeEventListener("contextmenu", close, true);
      document.removeEventListener("keydown", onKey);
    };
  });

  // ── Keyboard (row-level, not editing) ────────────────────────────────────

  function onRowKeyDown(e: KeyboardEvent) {
    if (mode === "editing") return;
    if (e.key === "Enter" || e.key === " ") { e.preventDefault(); enterEdit(); }
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") { e.preventDefault(); applyValue(current - (step || 0.01)); }
    else if (e.key === "ArrowRight" || e.key === "ArrowUp") { e.preventDefault(); applyValue(current + (step || 0.01)); }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="scrub-row"
  role="spinbutton"
  tabindex="0"
  aria-label={label}
  aria-valuenow={current}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuetext={displayValue}
  onkeydown={onRowKeyDown}
  onmouseenter={() => setHint(hint)}
  onmouseleave={() => clearHint()}
  oncontextmenu={onContextMenu}
>
  <span class="scrub-label" title={label}>{label}</span>

  <div class="scrub-widget" class:is-scrubbing={mode === "scrubbing"}>
    <!-- Decrement -->
    <button
      class="step-btn step-dec"
      tabindex="-1"
      aria-label="Decrease {label}"
      disabled={atMin}
      onpointerdown={(e) => { e.preventDefault(); startStep(-(step || 0.01)); }}
      onpointerup={stopStep}
      onpointerleave={stopStep}
      onpointercancel={stopStep}
    ><ChevronLeft size={11} /></button>

    <!-- Draggable track -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="track"
      class:editing={mode === "editing"}
      onpointerdown={onTrackPointerDown}
      onpointermove={onTrackPointerMove}
      onpointerup={onTrackPointerUp}
      ondblclick={onTrackDblClick}
    >
      {#if fillPct !== null}
        <div class="track-fill" style="width: {fillPct}%"></div>
      {/if}

      {#if mode === "editing"}
        <input
          bind:this={inputEl}
          bind:value={editText}
          class="track-input"
          type="text"
          inputmode="decimal"
          onkeydown={onInputKeyDown}
          onblur={commitEdit}
          onclick={(e) => e.stopPropagation()}
          onpointerdown={(e) => e.stopPropagation()}
        />
      {:else}
        <span class="track-value">{displayValue}</span>
      {/if}
    </div>

    <!-- Increment -->
    <button
      class="step-btn step-inc"
      tabindex="-1"
      aria-label="Increase {label}"
      disabled={atMax}
      onpointerdown={(e) => { e.preventDefault(); startStep(step || 0.01); }}
      onpointerup={stopStep}
      onpointerleave={stopStep}
      onpointercancel={stopStep}
    ><ChevronRight size={11} /></button>
  </div>
</div>

{#if ctx.open}
  <Portal>
    {@const mx = Math.min(ctx.x, (globalThis.window?.innerWidth ?? 800) - 170)}
    {@const my = Math.min(ctx.y, (globalThis.window?.innerHeight ?? 600) - 80)}
    <div class="ctx-menu" style="left: {mx}px; top: {my}px;">
      {#if defaultValue !== undefined}
        {@const dv = defaultValue}
        <button
          class="ctx-item"
          onclick={() => { applyValue(dv); ctx = { ...ctx, open: false }; }}
        >
          <span>Reset to default</span>
          <span class="ctx-hint">{dv.toFixed(decimals)}{unit ? " " + unit : ""}</span>
        </button>
      {/if}
      <button
        class="ctx-item"
        onclick={async () => {
          await navigator.clipboard.writeText(current.toFixed(decimals)).catch(() => {});
          ctx = { ...ctx, open: false };
        }}
      >
        Copy value
      </button>
    </div>
  </Portal>
{/if}

<style>
  /* ── Row ──────────────────────────────────────────────────────────────────── */
  .scrub-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 24px;
    outline: none;
    user-select: none;
  }

  .scrub-row:focus-visible .scrub-widget {
    box-shadow: 0 0 0 2px var(--interactive-ring);
  }

  /* ── Label ────────────────────────────────────────────────────────────────── */
  .scrub-label {
    flex-shrink: 0;
    width: 68px;
    font-size: 11px;
    font-weight: 500;
    color: var(--text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    pointer-events: none;
  }

  /* ── Widget (− track +) ───────────────────────────────────────────────────── */
  .scrub-widget {
    flex: 1;
    display: flex;
    align-items: stretch;
    height: 22px;
    border: 1px solid var(--stroke-mid);
    border-radius: 3px;
    overflow: hidden;
    transition: border-color 0.1s ease;
  }

  .scrub-widget:hover {
    border-color: var(--stroke-hi);
  }

  .scrub-widget.is-scrubbing {
    border-color: oklch(0.80 0.16 250 / 45%);
  }

  /* ── Step buttons ─────────────────────────────────────────────────────────── */
  .step-btn {
    flex-shrink: 0;
    width: 20px;
    background: var(--fill-lo);
    border: none;
    color: var(--text-faint);
    cursor: pointer;
    transition: background 0.1s ease, color 0.1s ease;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
  }

  .step-btn:hover:not(:disabled) {
    background: var(--fill-mid);
    color: var(--text-mid);
  }

  .step-btn:active:not(:disabled) {
    background: var(--interactive-fill);
    color: var(--text-hi);
  }

  .step-btn:disabled {
    opacity: 0.25;
    cursor: default;
  }

  /* Lucide renders SVG as inline by default — block removes baseline offset */
  .step-btn :global(svg) {
    display: block;
    flex-shrink: 0;
  }

  .step-dec {
    border-right: 1px solid var(--stroke-lo);
  }

  .step-inc {
    border-left: 1px solid var(--stroke-lo);
  }

  /* ── Track ────────────────────────────────────────────────────────────────── */
  .track {
    flex: 1;
    position: relative;
    background: var(--fill-lo);
    cursor: ew-resize;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .track.editing {
    cursor: default;
  }

  .track-fill {
    position: absolute;
    left: 0;
    top: 0;
    height: 100%;
    background: var(--interactive-fill);
    pointer-events: none;
  }

  .track-value {
    position: relative; /* above fill */
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-mid);
    white-space: nowrap;
    pointer-events: none;
  }

  /* ── Inline edit input ────────────────────────────────────────────────────── */
  .track-input {
    position: relative;
    width: 100%;
    height: 100%;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-hi);
    background: var(--surface-4);
    border: none;
    outline: none;
    text-align: center;
    padding: 0 4px;
  }

  /* ── Context menu ─────────────────────────────────────────────────────────── */
  .ctx-menu {
    position: fixed;
    z-index: 9999;
    background: var(--surface-5);
    border: 1px solid var(--stroke-mid);
    border-radius: 4px;
    padding: 3px;
    min-width: 164px;
    box-shadow: 0 4px 20px oklch(0 0 0 / 35%);
  }

  .ctx-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: 5px 8px;
    background: none;
    border: none;
    border-radius: 2px;
    cursor: pointer;
    font-size: 12px;
    color: var(--text-mid);
    text-align: left;
    gap: 12px;
  }

  .ctx-item:hover {
    background: var(--interactive-fill);
  }

  .ctx-hint {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-subtle);
    white-space: nowrap;
    flex-shrink: 0;
  }
</style>
