<script lang="ts">
  import { onMount } from "svelte";
  import * as RC from "../../../renderer/RendererController";

  let containerEl: HTMLDivElement;

  // Orbit camera state
  let orbitYaw = -0.7;
  let orbitPitch = 0.4;
  let orbitDist = 120;
  const target = [32, 32, 32];
  let dragging = false;

  function sendCamera() {
    orbitPitch = Math.max(-1.5, Math.min(1.5, orbitPitch));
    orbitDist = Math.max(10, Math.min(500, orbitDist));

    const cp = Math.cos(orbitPitch);
    const sp = Math.sin(orbitPitch);
    const cy = Math.cos(orbitYaw);
    const sy = Math.sin(orbitYaw);

    const px = target[0] + orbitDist * cp * sy;
    const py = target[1] + orbitDist * sp;
    const pz = target[2] + orbitDist * cp * cy;

    const dx = target[0] - px;
    const dy = target[1] - py;
    const dz = target[2] - pz;
    const len = Math.sqrt(dx * dx + dy * dy + dz * dz) || 1;

    RC.setCamera(px, py, pz, dx / len, dy / len, dz / len);
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0 && e.button !== 1) return;
    dragging = true;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    orbitYaw += e.movementX * 0.005;
    orbitPitch += e.movementY * 0.005;
    sendCamera();
  }

  function onPointerUp(e: PointerEvent) {
    dragging = false;
    (e.target as HTMLElement).releasePointerCapture(e.pointerId);
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    orbitDist += e.deltaY * 0.1;
    sendCamera();
  }

  onMount(() => {
    const canvas = containerEl.querySelector("canvas") as HTMLCanvasElement;

    // Initialize WASM renderer directly on main thread (ADR-0014)
    RC.init(canvas).then(() => {
      sendCamera();
    }).catch((err) => {
      console.error("[Viewport] renderer init failed:", err);
    });

    // Resize: ResizeObserver → direct WASM call
    const ro = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const { width, height } = entry.contentRect;
      if (RC.isReady() && width > 0 && height > 0) {
        const w = Math.round(width * devicePixelRatio);
        const h = Math.round(height * devicePixelRatio);
        canvas.width = w;
        canvas.height = h;
        RC.resize(w, h);
      }
    });
    ro.observe(containerEl);

    return () => {
      ro.disconnect();
      RC.destroy();
    };
  });
</script>

<div class="viewport" bind:this={containerEl}>
  <canvas
    id="gestalt-viewport"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onwheel={onWheel}
  ></canvas>
</div>

<style>
  .viewport {
    flex: 1;
    position: relative;
    overflow: hidden;
    background: var(--surface-1);
    min-width: 0;
    min-height: 0;
    width: 100%;
    height: 100%;
  }

  canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
</style>
