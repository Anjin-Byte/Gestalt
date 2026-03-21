<script lang="ts">
  import ScenePanel from "../panels/ScenePanel.svelte";
  import GpuPoolPanel from "../panels/GpuPoolPanel.svelte";
  import EditProtocolPanel from "../panels/EditProtocolPanel.svelte";
  import PerformancePanel from "../panels/PerformancePanel.svelte";
  import DebugPanel from "../panels/DebugPanel.svelte";
  import SettingsPanel from "../panels/SettingsPanel.svelte";
  import DemoPanel from "../panels/DemoPanel.svelte";

  let { activePanel }: { activePanel: string } = $props();

  const MIN_WIDTH = 180;
  const MAX_WIDTH = 520;

  let panelWidth = $state(280);
  let resizing = $state(false);
  let resizeStartX = 0;
  let resizeStartWidth = 0;

  function onResizePointerDown(e: PointerEvent) {
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    resizeStartX = e.clientX;
    resizeStartWidth = panelWidth;
    resizing = true;
  }

  function onResizePointerMove(e: PointerEvent) {
    if (!resizing) return;
    const dx = e.clientX - resizeStartX;
    panelWidth = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, resizeStartWidth + dx));
  }

  function onResizePointerUp(e: PointerEvent) {
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    resizing = false;
  }

  const collapsed = $derived(activePanel === "");
</script>

<div
  class="panel-area"
  class:collapsed
  class:resizing
  style={collapsed ? "" : `width: ${panelWidth}px; min-width: ${panelWidth}px;`}
>
  {#if activePanel === "scene"}
    <div class="panel-tab-content"><ScenePanel /></div>
  {:else if activePanel === "pool"}
    <div class="panel-tab-content"><GpuPoolPanel /></div>
  {:else if activePanel === "proto"}
    <div class="panel-tab-content"><EditProtocolPanel /></div>
  {:else if activePanel === "perf"}
    <div class="panel-tab-content"><PerformancePanel /></div>
  {:else if activePanel === "debug"}
    <div class="panel-tab-content"><DebugPanel /></div>
  {:else if activePanel === "settings"}
    <div class="panel-tab-content"><SettingsPanel /></div>
  {:else if activePanel === "demo"}
    <div class="panel-tab-content"><DemoPanel /></div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  {#if !collapsed}
    <div
      class="resize-handle"
      onpointerdown={onResizePointerDown}
      onpointermove={onResizePointerMove}
      onpointerup={onResizePointerUp}
      onpointercancel={onResizePointerUp}
    ></div>
  {/if}
</div>

<style>
  .panel-tab-content {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .resize-handle {
    position: absolute;
    right: -3px;
    top: 0;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 10;
  }

  .resize-handle:hover {
    background: var(--stroke-mid);
  }

  .resize-handle:active {
    background: var(--stroke-strong);
  }
</style>
