<script lang="ts">
  import type { Snippet } from "svelte";
  import DockTabs from "./DockTabs.svelte";
  import DropOverlay from "./DropOverlay.svelte";
  import { detectZone, hasPanelDrag, getDragPayload, type DropZonePosition } from "./dnd";

  let {
    groupId,
    panels,
    activePanel,
    panel,
    onactivate,
    onclose,
    dragActive = false,
    ondragstartpanel,
    ondragendpanel,
    ondragover: onDragOverGroup,
    ondragleave: onDragLeaveGroup,
    ondrop: onDropGroup,
  }: {
    groupId: string;
    panels: string[];
    activePanel: string;
    panel: Snippet<[string]>;
    onactivate: (groupId: string, panelId: string) => void;
    onclose?: (groupId: string, panelId: string) => void;
    /** Whether any panel drag is currently active (from DockLayout). */
    dragActive?: boolean;
    ondragstartpanel?: (groupId: string, panelId: string, event: DragEvent) => void;
    ondragendpanel?: () => void;
    ondragover?: (groupId: string, zone: DropZonePosition) => void;
    ondragleave?: (groupId: string) => void;
    ondrop?: (groupId: string, zone: DropZonePosition) => void;
  } = $props();

  let containerEl: HTMLDivElement | undefined = $state();
  let currentZone = $state<DropZonePosition | null>(null);
  let isTarget = $state(false);

  function handleDragOver(e: DragEvent) {
    if (!e.dataTransfer || !hasPanelDrag(e.dataTransfer)) return;
    e.preventDefault(); // Required for drop to fire
    e.dataTransfer.dropEffect = "move";

    if (!containerEl) return;
    const rect = containerEl.getBoundingClientRect();
    const zone = detectZone(rect, e.clientX, e.clientY);
    currentZone = zone;
    isTarget = true;
    onDragOverGroup?.(groupId, zone);
  }

  function handleDragLeave(e: DragEvent) {
    // Only clear if leaving the group entirely (not entering a child)
    if (containerEl && e.relatedTarget && containerEl.contains(e.relatedTarget as Node)) return;
    currentZone = null;
    isTarget = false;
    onDragLeaveGroup?.(groupId);
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    if (currentZone) {
      onDropGroup?.(groupId, currentZone);
    }
    currentZone = null;
    isTarget = false;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="dock-group"
  data-group-id={groupId}
  bind:this={containerEl}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  <DockTabs
    {panels}
    {activePanel}
    {groupId}
    onactivate={(panelId) => onactivate(groupId, panelId)}
    onclose={onclose ? (panelId) => onclose?.(groupId, panelId) : undefined}
    ondragstartpanel={(panelId, event) => ondragstartpanel?.(groupId, panelId, event)}
    ondragendpanel={ondragendpanel}
  />
  <div class="dock-group-content">
    {#key activePanel}
      {@render panel(activePanel)}
    {/key}
  </div>

  {#if dragActive && isTarget}
    <DropOverlay zone={currentZone} />
  {/if}
</div>

<style>
  .dock-group {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--surface-3, oklch(0.18 0.015 250));
    border: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.06));
    box-sizing: border-box;
    position: relative; /* For DropOverlay absolute positioning */
  }

  .dock-group-content {
    flex: 1;
    overflow: auto;
    min-width: 0;
    min-height: 0;
  }
</style>
