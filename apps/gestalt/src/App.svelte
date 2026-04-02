<script lang="ts">
  import { DockLayout, Gridview } from "@gestalt/phi";
  import type { DockPanelGroup, IGridView } from "@gestalt/phi";
  import StatusBar from "$lib/components/shell/StatusBar.svelte";
  import Viewport from "$lib/components/shell/Viewport.svelte";
  import InspectorPanel from "$lib/components/panels/InspectorPanel.svelte";
  import PerformancePanel from "$lib/components/panels/PerformancePanel.svelte";
  import DemoPanel from "$lib/components/panels/DemoPanel.svelte";

  // ─── Grid View Factory ─────────────────────────────────────────────────

  function createPanelView(id: string): IGridView & { id: string } {
    return {
      id,
      minimumWidth: 180,
      maximumWidth: Number.POSITIVE_INFINITY,
      minimumHeight: 100,
      maximumHeight: Number.POSITIVE_INFINITY,
      layout() {},
    };
  }

  // ─── Default Layout ────────────────────────────────────────────────────
  //
  //  ┌───────────┬────────────────────┬───────────┐
  //  │           │                    │           │
  //  │ Inspector │     Viewport       │   Perf    │
  //  │           │                    │           │
  //  │           │                    ├───────────┤
  //  │           │                    │   Demo    │
  //  │           │                    │(reference)│
  //  └───────────┴────────────────────┴───────────┘

  function createDefaultLayout() {
    const gv = new Gridview("horizontal");

    // Left: inspector (narrow sidebar)
    gv.addView(createPanelView("left"), 220, [0]);
    // Center: viewport (takes most of the width)
    gv.addView(createPanelView("viewport"), 800, [1]);
    // Right: perf (narrow sidebar) — split vertically with demo below
    gv.addViewAt(createPanelView("right"), 220, "right", [1]);
    // Demo takes a small slice at the bottom-right (reference only)
    gv.addViewAt(createPanelView("right-bottom"), 160, "down", [2]);

    const groups: Record<string, DockPanelGroup> = {
      left:           { id: "left",          panels: ["inspector"],  activePanel: "inspector" },
      viewport:       { id: "viewport",      panels: ["viewport"],   activePanel: "viewport" },
      right:          { id: "right",         panels: ["perf"],       activePanel: "perf" },
      "right-bottom": { id: "right-bottom",  panels: ["demo"],       activePanel: "demo" },
    };

    return { gv, groups };
  }

  const WORKSPACE_KEY = "gestalt-workspace";

  function restoreOrCreateLayout() {
    const saved = localStorage.getItem(WORKSPACE_KEY);
    if (saved) {
      try {
        const data = JSON.parse(saved);
        const gv = Gridview.deserialize(data.grid, (id: string) => createPanelView(id));
        return { gv, groups: data.groups as Record<string, DockPanelGroup> };
      } catch (e) {
        console.warn("[App] corrupt workspace, falling back to default:", e);
        localStorage.removeItem(WORKSPACE_KEY);
      }
    }
    return createDefaultLayout();
  }

  const { gv: gridview, groups: defaultGroups } = restoreOrCreateLayout();
  let groups = $state<Record<string, DockPanelGroup>>(defaultGroups);

  function handleChange() {
    try {
      const data = {
        grid: gridview.serialize(),
        groups: groups,
      };
      localStorage.setItem(WORKSPACE_KEY, JSON.stringify(data));
    } catch {
      // localStorage full or unavailable — silently ignore
    }
  }

  $effect(() => {
    document.body.dataset.ready = "true";
  });
</script>

<div class="dark app-shell">
  <DockLayout
    {gridview}
    bind:groups
    panel={panelSnippet}
    onchange={handleChange}
    createView={createPanelView}
  />
  <StatusBar />
</div>

{#snippet panelSnippet(panelId: string)}
  <div class="dock-panel-wrap" class:dock-panel-viewport={panelId === "viewport"}>
    {#if panelId === "viewport"}
      <Viewport />
    {:else if panelId === "inspector"}
      <InspectorPanel />
    {:else if panelId === "perf"}
      <PerformancePanel />
    {:else if panelId === "demo"}
      <DemoPanel />
    {:else}
      <div class="dock-panel-empty">{panelId}</div>
    {/if}
  </div>
{/snippet}

<style>
  .app-shell {
    display: flex;
    flex-direction: column;
    height: 100dvh;
    width: 100%;
    overflow: hidden;
  }

  .dock-panel-wrap {
    width: 100%;
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0 10px;
  }

  .dock-panel-viewport {
    padding: 0;
    overflow: hidden;
  }

  .dock-panel-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    font-size: 12px;
    color: var(--text-faint);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
</style>
