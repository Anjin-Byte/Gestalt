<script lang="ts">
  /**
   * Tab bar for a dock group. Handles tab switching, close, and drag initiation.
   * Each tab is draggable — starting a drag fires ondragstartpanel so DockLayout
   * can track the operation and render drop zone overlays on target groups.
   */
  import { setDragPayload } from "./dnd";

  let {
    panels,
    activePanel,
    groupId,
    onactivate,
    onclose,
    ondragstartpanel,
    ondragendpanel,
  }: {
    panels: string[];
    activePanel: string;
    /** Group ID this tab bar belongs to. Needed for drag payload. */
    groupId: string;
    onactivate: (panelId: string) => void;
    onclose?: (panelId: string) => void;
    /** Fired when a tab drag begins. */
    ondragstartpanel?: (panelId: string, event: DragEvent) => void;
    /** Fired when a tab drag ends (regardless of drop). */
    ondragendpanel?: () => void;
  } = $props();

  function handleDragStart(panelId: string, event: DragEvent) {
    if (!event.dataTransfer) return;
    setDragPayload(event.dataTransfer, { panelId, sourceGroupId: groupId });
    // Ghost image: use the tab element itself
    const target = event.currentTarget as HTMLElement;
    event.dataTransfer.setDragImage(target, 20, 13);
    ondragstartpanel?.(panelId, event);
  }

  function handleDragEnd() {
    ondragendpanel?.();
  }
</script>

<div class="dock-tabs" role="tablist">
  {#each panels as panel}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="dock-tab"
      class:active={panel === activePanel}
      role="tab"
      aria-selected={panel === activePanel}
      draggable="true"
      onclick={() => onactivate(panel)}
      ondragstart={(e) => handleDragStart(panel, e)}
      ondragend={handleDragEnd}
    >
      <span class="dock-tab-label">{panel}</span>
      {#if onclose}
        <button
          class="dock-tab-close"
          aria-label="Close {panel}"
          onclick={(e) => {
            e.stopPropagation();
            onclose?.(panel);
          }}
        >&times;</button>
      {/if}
    </div>
  {/each}
</div>

<style>
  .dock-tabs {
    display: flex;
    align-items: stretch;
    height: 26px;
    background: var(--fill-lo, oklch(1 0 0 / 0.05));
    border-bottom: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.06));
    overflow-x: auto;
    overflow-y: hidden;
    flex-shrink: 0;
    user-select: none;
  }

  .dock-tab {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 10px;
    border: none;
    border-right: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.06));
    background: none;
    color: var(--text-subtle, #999);
    font-size: 11px;
    font-weight: 500;
    cursor: grab;
    white-space: nowrap;
    transition: color 0.1s ease, background 0.1s ease;
  }

  .dock-tab:hover {
    color: var(--text-mid, #ccc);
    background: var(--fill-mid, oklch(1 0 0 / 0.08));
  }

  .dock-tab.active {
    color: var(--text-hi, #eee);
    background: var(--fill-mid, oklch(1 0 0 / 0.08));
    border-bottom: 2px solid var(--interactive, oklch(0.80 0.16 250));
  }

  .dock-tab:active {
    cursor: grabbing;
  }

  .dock-tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dock-tab-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    padding: 0;
    border: none;
    border-radius: 2px;
    background: none;
    color: var(--text-faint, #555);
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
  }

  .dock-tab-close:hover {
    color: var(--text-mid, #ccc);
    background: oklch(1 0 0 / 0.1);
  }
</style>
