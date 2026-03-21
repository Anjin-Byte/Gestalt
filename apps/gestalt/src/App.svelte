<script lang="ts">
  import Sidebar from "$lib/components/shell/Sidebar.svelte";
  import PanelArea from "$lib/components/shell/PanelArea.svelte";
  import Viewport from "$lib/components/shell/Viewport.svelte";
  import StatusBar from "$lib/components/shell/StatusBar.svelte";
  import { RendererBridge } from "./renderer/RendererBridge";
  import { rendererBridgeStore } from "$lib/stores/rendererBridge";

  let activePanel = $state("scene");

  function onTabClick(value: string) {
    activePanel = activePanel === value ? "" : value;
  }

  // Initialize the renderer worker bridge.
  // Resolves to null when COOP/COEP headers are absent (SAB unavailable),
  // or when the worker fails to initialize — the app continues without it.
  $effect(() => {
    let bridge: RendererBridge | null = null;

    RendererBridge.create().then((b) => {
      bridge = b;
      rendererBridgeStore.set(b);
    });

    return () => {
      bridge?.terminate();
      rendererBridgeStore.set(null);
    };
  });

  // Signal to Playwright that the shell has mounted.
  $effect(() => {
    document.body.dataset.ready = "true";
  });
</script>

<div class="dark app-shell">
  <div class="layout">
    <Sidebar {activePanel} {onTabClick} />
    <PanelArea {activePanel} />
    <Viewport />
  </div>
  <StatusBar />
</div>

<style>
  .app-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    width: 100%;
  }
</style>
