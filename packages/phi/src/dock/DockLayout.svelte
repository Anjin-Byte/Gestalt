<script lang="ts" module>
  /** Panel group state — stored per leaf in the Gridview. */
  export interface DockPanelGroup {
    id: string;
    panels: string[];
    activePanel: string;
  }
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import { Gridview, LeafNode, BranchNode, type IGridView } from "./Gridview";
  import { type Orientation } from "./Splitview";
  import DockGroup from "./DockGroup.svelte";
  import { type DropZonePosition, type PanelDragPayload, zoneToDirection } from "./dnd";

  let {
    gridview,
    panel,
    groups = $bindable(),
    onchange,
    createView,
  }: {
    gridview: Gridview;
    panel: Snippet<[string]>;
    groups: Record<string, DockPanelGroup>;
    onchange?: () => void;
    /** Factory for creating IGridView instances (needed for edge drops that create new splits). */
    createView?: (id: string) => IGridView & { id: string };
  } = $props();

  // ─── Container + ResizeObserver ──────────────────────────────────────

  let containerEl = $state<HTMLElement | null>(null);
  let containerW = $state(0);
  let containerH = $state(0);
  let layoutVersion = $state(0);

  onMount(() => {
    if (!containerEl) return;
    const ro = new ResizeObserver((entries) => {
      const { width, height } = entries[0].contentRect;
      if (width > 0 && height > 0) {
        containerW = width;
        containerH = height;
        gridview.layout(width, height);
        layoutVersion++;
      }
    });
    ro.observe(containerEl);
    const rect = containerEl.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      containerW = rect.width;
      containerH = rect.height;
      gridview.layout(rect.width, rect.height);
      layoutVersion++;
    }
    return () => ro.disconnect();
  });

  function relayout() {
    if (containerW > 0 && containerH > 0) {
      gridview.layout(containerW, containerH);
      layoutVersion++;
    }
    onchange?.();
  }

  // ─── Tab activation ────────────────────────────────────────────────

  function handleActivate(groupId: string, panelId: string) {
    const group = groups[groupId];
    if (group) {
      group.activePanel = panelId;
      groups = { ...groups };
      onchange?.();
    }
  }

  function handleClose(groupId: string, panelId: string) {
    const group = groups[groupId];
    if (!group) return;

    group.panels = group.panels.filter((p) => p !== panelId);

    if (group.panels.length === 0) {
      // Merge: remove empty group from grid tree
      removeGroupFromTree(groupId);
      return;
    }

    if (group.activePanel === panelId) {
      group.activePanel = group.panels[0];
    }
    groups = { ...groups };
    onchange?.();
  }

  // ─── DnD state ───────────────────────────────────────────────────

  let dragActive = $state(false);
  let dragPayload = $state<PanelDragPayload | null>(null);

  function handleDragStartPanel(groupId: string, panelId: string, _event: DragEvent) {
    dragActive = true;
    dragPayload = { panelId, sourceGroupId: groupId };
  }

  function handleDragEndPanel() {
    dragActive = false;
    dragPayload = null;
  }

  function handleGroupDragOver(_groupId: string, _zone: DropZonePosition) {
    // Zone tracking is handled locally in DockGroup. DockLayout just knows drag is active.
  }

  function handleGroupDragLeave(_groupId: string) {
    // Handled locally in DockGroup.
  }

  function handleGroupDrop(targetGroupId: string, zone: DropZonePosition) {
    if (!dragPayload) return;
    const { panelId, sourceGroupId } = dragPayload;

    // Don't drop on self with center (would be a no-op)
    if (sourceGroupId === targetGroupId && zone === "center") {
      handleDragEndPanel();
      return;
    }

    const direction = zoneToDirection(zone);

    if (direction === null) {
      // Center drop: tabify — move panel to target group
      movePanelToGroup(panelId, sourceGroupId, targetGroupId);
    } else {
      // Edge drop: split — create new group at direction
      splitAndMovePanel(panelId, sourceGroupId, targetGroupId, direction);
    }

    handleDragEndPanel();
    relayout();
  }

  // ─── Panel move operations ──────────────────────────────────────

  function movePanelToGroup(panelId: string, sourceGroupId: string, targetGroupId: string) {
    const source = groups[sourceGroupId];
    const target = groups[targetGroupId];
    if (!source || !target) return;

    // Remove from source
    source.panels = source.panels.filter((p) => p !== panelId);

    // Add to target
    target.panels.push(panelId);
    target.activePanel = panelId;

    // Clean up empty source
    if (source.panels.length === 0) {
      removeGroupFromTree(sourceGroupId);
    } else {
      if (source.activePanel === panelId) {
        source.activePanel = source.panels[0];
      }
      groups = { ...groups };
    }
  }

  function splitAndMovePanel(
    panelId: string,
    sourceGroupId: string,
    targetGroupId: string,
    direction: "up" | "down" | "left" | "right",
  ) {
    if (!createView) {
      console.warn("[DockLayout] createView prop required for edge drops (split)");
      return;
    }

    // Find target's path in the grid tree
    const targetPath = findGroupPath(targetGroupId);
    if (!targetPath) return;

    // Create new group
    const newGroupId = `group-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    const newView = createView(newGroupId);
    gridview.addViewAt(newView, 0.5, direction, targetPath);

    // Remove panel from source
    const source = groups[sourceGroupId];
    if (source) {
      source.panels = source.panels.filter((p) => p !== panelId);
      if (source.panels.length === 0) {
        removeGroupFromTree(sourceGroupId);
      } else if (source.activePanel === panelId) {
        source.activePanel = source.panels[0];
      }
    }

    // Register new group
    groups[newGroupId] = { id: newGroupId, panels: [panelId], activePanel: panelId };
    groups = { ...groups };
  }

  function removeGroupFromTree(groupId: string) {
    const path = findGroupPath(groupId);
    if (path) {
      gridview.removeView(path);
    }
    delete groups[groupId];
    groups = { ...groups };
  }

  function findGroupPath(groupId: string): number[] | null {
    // Walk the tree to find the leaf whose view.id === groupId
    function walk(node: BranchNode | LeafNode, path: number[]): number[] | null {
      if (node.kind === "leaf") {
        const id = (node.view as any)?.id;
        return id === groupId ? path : null;
      }
      for (let i = 0; i < node.children.length; i++) {
        const result = walk(node.children[i], [...path, i]);
        if (result) return result;
      }
      return null;
    }
    return walk(gridview.root, []);
  }

  // ─── Sash dragging ─────────────────────────────────────────────────

  let sashDragging = $state(false);

  function handleSashDown(branch: BranchNode, sashIndex: number, event: PointerEvent) {
    event.preventDefault();
    const el = event.currentTarget as HTMLElement;
    el.setPointerCapture(event.pointerId);
    sashDragging = true;

    const snapshot = branch.splitview.getSizes();
    const startPos = branch.splitview.orientation === "horizontal"
      ? event.clientX
      : event.clientY;

    const onMove = (e: PointerEvent) => {
      const currentPos = branch.splitview.orientation === "horizontal"
        ? e.clientX
        : e.clientY;
      const delta = currentPos - startPos;
      branch.splitview.resize(sashIndex, delta, snapshot);
      branch.splitview.distributeEmptySpace();
      const sizes = branch.splitview.getSizes();
      for (let i = 0; i < branch.children.length; i++) {
        const child = branch.children[i];
        const childSize = sizes[i];
        const orthSize = branch.splitview.orientation === "horizontal"
          ? containerH : containerW;
        child.layout(childSize, orthSize);
      }
      layoutVersion++;
    };

    const onUp = (e: PointerEvent) => {
      el.releasePointerCapture(e.pointerId);
      el.removeEventListener("pointermove", onMove);
      el.removeEventListener("pointerup", onUp);
      branch.splitview.saveProportions();
      sashDragging = false;
      onchange?.();
    };

    el.addEventListener("pointermove", onMove);
    el.addEventListener("pointerup", onUp);
  }

  // ─── Recursive layout computation ──────────────────────────────────

  interface LayoutRect {
    left: number;
    top: number;
    width: number;
    height: number;
  }

  interface LeafLayout {
    kind: "leaf";
    groupId: string;
    rect: LayoutRect;
  }

  interface SashLayout {
    branch: BranchNode;
    sashIndex: number;
    rect: LayoutRect;
    orientation: Orientation;
  }

  function computeLayout(
    node: BranchNode | LeafNode,
    left: number,
    top: number,
    width: number,
    height: number,
  ): { leaves: LeafLayout[]; sashes: SashLayout[] } {
    void layoutVersion;

    if (node.kind === "leaf") {
      const group = (node.view as any)?.id ?? "";
      return {
        leaves: [{ kind: "leaf", groupId: group, rect: { left, top, width, height } }],
        sashes: [],
      };
    }

    const branch = node;
    const sizes = branch.splitview.getSizes();
    const isH = branch.splitview.orientation === "horizontal";
    const leaves: LeafLayout[] = [];
    const sashes: SashLayout[] = [];
    let offset = 0;

    for (let i = 0; i < branch.children.length; i++) {
      const childSize = sizes[i] ?? 0;
      const childLeft = isH ? left + offset : left;
      const childTop = isH ? top : top + offset;
      const childWidth = isH ? childSize : width;
      const childHeight = isH ? height : childSize;

      const childResult = computeLayout(
        branch.children[i],
        childLeft, childTop, childWidth, childHeight,
      );
      leaves.push(...childResult.leaves);
      sashes.push(...childResult.sashes);

      if (i < branch.children.length - 1) {
        const sashThickness = 4;
        sashes.push({
          branch,
          sashIndex: i,
          orientation: branch.splitview.orientation,
          rect: isH
            ? { left: left + offset + childSize - sashThickness / 2, top, width: sashThickness, height }
            : { left, top: top + offset + childSize - sashThickness / 2, width, height: sashThickness },
        });
      }

      offset += childSize;
    }

    return { leaves, sashes };
  }

  const layout = $derived((() => {
    void layoutVersion;
    if (containerW === 0 || containerH === 0) return { leaves: [], sashes: [] };
    return computeLayout(gridview.root, 0, 0, containerW, containerH);
  })());
</script>

<div
  class="dock-layout"
  class:resizing={sashDragging}
  bind:this={containerEl}
>
  {#each layout.leaves as leaf (leaf.groupId)}
    {@const group = groups[leaf.groupId]}
    {#if group}
      <div
        class="dock-leaf"
        style="left:{leaf.rect.left}px;top:{leaf.rect.top}px;width:{leaf.rect.width}px;height:{leaf.rect.height}px;"
      >
        <DockGroup
          groupId={group.id}
          panels={group.panels}
          activePanel={group.activePanel}
          {panel}
          {dragActive}
          onactivate={handleActivate}
          onclose={handleClose}
          ondragstartpanel={handleDragStartPanel}
          ondragendpanel={handleDragEndPanel}
          ondragover={handleGroupDragOver}
          ondragleave={handleGroupDragLeave}
          ondrop={handleGroupDrop}
        />
      </div>
    {/if}
  {/each}

  {#each layout.sashes as sash, i (i)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="dock-sash"
      class:dock-sash-h={sash.orientation === "horizontal"}
      class:dock-sash-v={sash.orientation === "vertical"}
      style="left:{sash.rect.left}px;top:{sash.rect.top}px;width:{sash.rect.width}px;height:{sash.rect.height}px;"
      onpointerdown={(e) => handleSashDown(sash.branch, sash.sashIndex, e)}
    ></div>
  {/each}
</div>

<style>
  .dock-layout {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }

  .dock-layout.resizing {
    user-select: none;
    cursor: col-resize;
  }

  .dock-leaf {
    position: absolute;
    overflow: hidden;
  }

  .dock-sash {
    position: absolute;
    z-index: 10;
    background: transparent;
    transition: background 0.1s ease;
  }

  .dock-sash-h {
    cursor: ew-resize;
  }

  .dock-sash-v {
    cursor: ns-resize;
  }

  .dock-sash:hover {
    background: var(--interactive, oklch(0.80 0.16 250));
    opacity: 0.4;
  }

  .dock-sash:active {
    background: var(--interactive, oklch(0.80 0.16 250));
    opacity: 0.6;
  }
</style>
