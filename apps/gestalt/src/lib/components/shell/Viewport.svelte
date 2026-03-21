<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { viewerStore, backendStore, fpsText } from "$lib/stores/viewer";
  import { createThreeBackend } from "@web/viewer/threeBackend";
  import { Viewer } from "@web/viewer/Viewer";
  import { patchWebGpuLimits } from "@web/gpu/patchLimits";

  const savedPreference =
    (localStorage.getItem("rendererPreference") as "auto" | "webgpu" | "webgl" | null) ?? "auto";

  let canvasEl: HTMLCanvasElement;
  let rafId = 0;
  let frameCount = 0;
  let lastSampleTime = 0;

  const tick = (viewer: Viewer) => {
    rafId = requestAnimationFrame(() => tick(viewer));
    viewer.render();
    frameCount++;
    const now = performance.now();
    const delta = now - lastSampleTime;
    if (delta >= 500) {
      const fps = Math.round((frameCount / delta) * 1000);
      const ms = (delta / frameCount).toFixed(1);
      fpsText.set(`${fps} fps  ${ms} ms`);
      lastSampleTime = now;
      frameCount = 0;
    }
  };

  onMount(async () => {
    patchWebGpuLimits();

    const backend = await createThreeBackend(canvasEl, {
      testMode: false,
      preferredRenderer: savedPreference,
    });

    const viewer = new Viewer(backend, { overlay: document.createElement("span"), testMode: false });

    const rect = canvasEl.getBoundingClientRect();
    viewer.resize(rect.width, rect.height);

    viewerStore.set(viewer);
    backendStore.set(backend);

    lastSampleTime = performance.now();
    tick(viewer);

    const onResize = () => {
      const r = canvasEl.getBoundingClientRect();
      viewer.resize(r.width, r.height);
    };
    window.addEventListener("resize", onResize);

    return () => window.removeEventListener("resize", onResize);
  });

  onDestroy(() => {
    if (rafId) cancelAnimationFrame(rafId);
    viewerStore.set(null);
    backendStore.set(null);
  });
</script>

<div class="viewport">
  <canvas bind:this={canvasEl}></canvas>
</div>

<style>
  .viewport {
    flex: 1;
    position: relative;
    overflow: hidden;
    background: var(--surface-1);
    min-width: 0;
  }

  canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
</style>
