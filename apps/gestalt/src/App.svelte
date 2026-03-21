<script lang="ts">
  import { ModuleHost } from "@gestalt/modules";
  import type { ModuleOutput } from "@gestalt/modules";
  import { viewerStore } from "$lib/stores/viewer";
  import { createSvelteUiApi, scheduleRun } from "$lib/stores/moduleControls";
  import { requestGpuDevice } from "$lib/utils/gpu";
  import { stubModule } from "./modules/stub";
  import Sidebar from "$lib/components/shell/Sidebar.svelte";
  import PanelArea from "$lib/components/shell/PanelArea.svelte";
  import Viewport from "$lib/components/shell/Viewport.svelte";
  import StatusBar from "$lib/components/shell/StatusBar.svelte";

  let activePanel = $state("scene");
  let host = $state<ModuleHost | null>(null);

  const modules = [stubModule];
  const uiApi = createSvelteUiApi();

  const logger = {
    info: (m: string) => console.info(m),
    warn: (m: string) => console.warn(m),
    error: (m: string) => console.error(m),
  };

  function onTabClick(value: string) {
    activePanel = activePanel === value ? "" : value;
  }

  $effect(() => {
    const viewer = $viewerStore;
    if (!viewer) return;

    const ctx = {
      requestGpuDevice,
      logger,
      baseUrl: import.meta.env.BASE_URL,
      appendOutputs: (outputs: ModuleOutput[]) => viewer.appendOutputs(outputs),
      clearChunkOutputs: () => viewer.clearChunkOutputs(),
    };

    let newHost: ModuleHost;
    newHost = new ModuleHost(modules, uiApi, ctx, (outputs) => {
      viewer.setOutputs(outputs);
    });

    host = newHost;
    scheduleRun.set(() => newHost.scheduleRun());
    void newHost.activate(modules[0].id);

    return () => {
      void newHost.dispose();
      host = null;
      scheduleRun.set(() => {});
    };
  });
</script>

<div class="dark app-shell">
  <div class="layout">
    <Sidebar {activePanel} {onTabClick} />
    <PanelArea {host} {activePanel} />
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
