<script lang="ts">
  import { onMount } from "svelte";
  import type { TreeListDomain, DropZone } from "./types";
  import type { ContextMenuItem } from "../ContextMenu.svelte";
  import { TreeListStateStore } from "./types";
  import TreeListRow from "./TreeListRow.svelte";
  import TreeListGroup from "./TreeListGroup.svelte";
  import ContextMenu from "../ContextMenu.svelte";

  let {
    domain,
    data,
    maxRows = 200,
    selectedId = null,
    activeId = null,
    onselectionchange,
  }: {
    domain: TreeListDomain<any>;
    data: any;
    maxRows?: number;
    /** Externally-controlled selected row ID. */
    selectedId?: string | null;
    /** Externally-controlled active row ID. */
    activeId?: string | null;
    /** Notifies parent when selection changes from user interaction. */
    onselectionchange?: (selectedId: string | null, activeId: string | null) => void;
  } = $props();

  // ─── State ─────────────────────────────────────────────────────────────

  // Capture domainId at init to avoid Svelte's "only captures initial value" warning.
  const domainId = domain.domainId;
  const stateStore = new TreeListStateStore(domainId);

  // Reactive version counter — incremented whenever stateStore mutates,
  // so $derived computations that read it will re-run.
  let collapseVersion = $state(0);

  let filterString = $state("");
  let scrollEl = $state<HTMLElement | null>(null);
  // Reserved for future container-query width tracking via ResizeObserver.
  // let containerWidth = $state(0);

  // Drag state
  let dragSourceId = $state<string | null>(null);
  let dropTarget = $state<{ id: string; zone: DropZone } | null>(null);

  // Inline rename state
  let editingId = $state<string | null>(null);

  // Context menu state
  let contextMenu = $state<{
    rowId: string;
    items: ContextMenuItem[];
    x: number;
    y: number;
  } | null>(null);

  // Row element refs for scroll-to-active
  const rowElements = new Map<string, HTMLElement>();

  /** Toggle a group and bump the reactive version so derived lists re-compute. */
  function toggleGroup(groupId: string) {
    stateStore.toggle(groupId);
    collapseVersion++;
  }

  function expandGroup(groupId: string) {
    stateStore.expand(groupId);
    collapseVersion++;
  }

  function collapseGroup(groupId: string) {
    stateStore.collapse(groupId);
    collapseVersion++;
  }

  // ─── Scroll persistence ────────────────────────────────────────────────

  const savedScrollTop: Record<string, number> = {};

  onMount(() => {
    if (scrollEl) {
      scrollEl.scrollTop = savedScrollTop[domainId] ?? 0;
    }
    return () => {
      if (scrollEl) {
        savedScrollTop[domainId] = scrollEl.scrollTop;
      }
    };
  });

  // ─── Derived collapse state (reactive) ─────────────────────────────────

  /** Reactive set of group open states, re-derived whenever collapseVersion bumps. */
  const groupOpenState = $derived((() => {
    void collapseVersion;
    const map = new Map<string, boolean>();
    for (const item of domain.rows(data)) {
      if (item.kind === "group") {
        map.set(item.id, !stateStore.isCollapsed(item.id));
      }
    }
    return map;
  })());

  // ─── Derived item list ─────────────────────────────────────────────────

  const allItems = $derived(domain.rows(data));

  const visibleItems = $derived((() => {
    // Reference collapseVersion so Svelte re-derives when collapse state changes.
    void collapseVersion;
    const collapsed = stateStore.collapsedSet;
    const filter = filterString.trim().toLowerCase();

    if (!filter) {
      // No filter — just hide rows under collapsed groups
      return allItems.filter((item) => {
        if (item.kind === "group") return true;
        return !collapsed.has(item.groupId);
      });
    }

    // With filter: find matching rows, then determine visible groups
    const matchingGroupIds = new Set<string>();
    const matchingRowIds = new Set<string>();

    for (const item of allItems) {
      if (item.kind === "row" && item.label.toLowerCase().includes(filter)) {
        matchingRowIds.add(item.id);
        matchingGroupIds.add(item.groupId);
      }
    }

    return allItems.filter((item) => {
      if (item.kind === "group") return matchingGroupIds.has(item.id);
      return matchingRowIds.has(item.id);
    });
  })());

  const displayItems = $derived(
    visibleItems.length > maxRows
      ? visibleItems.slice(0, maxRows)
      : visibleItems
  );

  const overflowCount = $derived(
    Math.max(0, visibleItems.length - maxRows)
  );

  // Indices of visible row items (not groups) for keyboard navigation
  const navigableIndices = $derived(
    displayItems.reduce<number[]>((acc, item, i) => {
      if (item.kind === "row") acc.push(i);
      return acc;
    }, [])
  );

  // ─── Selection ─────────────────────────────────────────────────────────

  function selectRow(id: string) {
    onselectionchange?.(id, id);
    domain.onSelect?.(id);
  }

  function clearSelection() {
    onselectionchange?.(null, null);
  }

  function handleRowClick(id: string, event: MouseEvent) {
    // Double-click enters rename mode (Blender pattern)
    if (event.detail === 2) {
      startRename(id);
      return;
    }
    selectRow(id);
  }

  // ─── Inline Rename ─────────────────────────────────────────────────────

  function startRename(id: string) {
    if (!domain.onRename) return;
    const item = allItems.find((i) => i.id === id);
    if (item?.kind !== "row" || !item.renameable) return;
    editingId = id;
  }

  function handleRename(id: string, newLabel: string) {
    domain.onRename?.(id, newLabel);
  }

  function handleRenameComplete() {
    editingId = null;
  }

  // ─── Delete ────────────────────────────────────────────────────────────

  function handleDelete() {
    if (!domain.onDelete || !selectedId) return;
    domain.onDelete([selectedId]);
  }

  // ─── Duplicate ─────────────────────────────────────────────────────────

  function handleDuplicate() {
    if (!domain.onDuplicate || !selectedId) return;
    domain.onDuplicate([selectedId]);
  }

  // ─── Context Menu ───────────────────────────────────────────────────────

  function handleContextMenu(id: string, event: MouseEvent) {
    event.preventDefault();
    if (!domain.getContextItems) return;

    // Select the right-clicked row
    selectRow(id);

    const items = domain.getContextItems(id);
    if (items.length === 0) return;

    contextMenu = { rowId: id, items, x: event.clientX, y: event.clientY };
  }

  function handleContextAction(actionId: string) {
    if (!contextMenu) return;
    domain.onContextAction?.(contextMenu.rowId, actionId);
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  // ─── Keyboard ──────────────────────────────────────────────────────────

  function handleKeydown(event: KeyboardEvent) {
    const currentNavIdx = activeId
      ? navigableIndices.findIndex((i) => displayItems[i].id === activeId)
      : -1;

    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        const nextIdx = currentNavIdx < navigableIndices.length - 1 ? currentNavIdx + 1 : currentNavIdx;
        if (nextIdx >= 0 && nextIdx < navigableIndices.length) {
          const item = displayItems[navigableIndices[nextIdx]];
          selectRow(item.id);
          scrollRowIntoView(item.id);
        }
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        const prevIdx = currentNavIdx > 0 ? currentNavIdx - 1 : 0;
        if (prevIdx >= 0 && prevIdx < navigableIndices.length) {
          const item = displayItems[navigableIndices[prevIdx]];
          selectRow(item.id);
          scrollRowIntoView(item.id);
        }
        break;
      }
      case "ArrowRight": {
        event.preventDefault();
        // If active is a group header, expand it
        if (activeId) {
          const activeItem = allItems.find((i) => i.id === activeId);
          if (activeItem?.kind === "group" && stateStore.isCollapsed(activeItem.id)) {
            expandGroup(activeItem.id);
          }
        }
        break;
      }
      case "ArrowLeft": {
        event.preventDefault();
        if (activeId) {
          const activeItem = allItems.find((i) => i.id === activeId);
          // If active is an expanded group, collapse it
          if (activeItem?.kind === "group" && !stateStore.isCollapsed(activeItem.id)) {
            collapseGroup(activeItem.id);
          }
          // If active is a row, jump focus to its parent group
          else if (activeItem?.kind === "row") {
            selectRow(activeItem.groupId);
          }
        }
        break;
      }
      case "Escape": {
        event.preventDefault();
        if (contextMenu) {
          closeContextMenu();
        } else if (editingId) {
          editingId = null;
        } else if (filterString) {
          filterString = "";
        } else {
          clearSelection();
        }
        break;
      }
      case "F2": {
        event.preventDefault();
        if (activeId) {
          startRename(activeId);
        }
        break;
      }
      case "Delete":
      case "Backspace": {
        // Don't delete while renaming or filtering
        if (editingId) break;
        if (document.activeElement?.tagName === "INPUT") break;
        event.preventDefault();
        handleDelete();
        break;
      }
      case "d": {
        // Ctrl+D (Windows/Linux) or Cmd+D (Mac)
        if (event.ctrlKey || event.metaKey) {
          event.preventDefault();
          handleDuplicate();
        }
        break;
      }
    }
  }

  function scrollRowIntoView(id: string) {
    const el = rowElements.get(id);
    el?.scrollIntoView({ block: "nearest" });
  }

  // ─── Show Active (public API) ──────────────────────────────────────────

  export function showActive() {
    if (!activeId) return;

    // Force-expand ancestors
    const activeItem = allItems.find((i) => i.id === activeId);
    if (activeItem?.kind === "row") {
      expandGroup(activeItem.groupId);
    }

    // Wait for DOM update, then scroll
    requestAnimationFrame(() => {
      const el = rowElements.get(activeId!);
      el?.scrollIntoView({ block: "center", behavior: "smooth" });
    });
  }

  // ─── Drag & Drop ──────────────────────────────────────────────────────

  function handleDragStart(id: string, event: DragEvent) {
    dragSourceId = id;
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", id);
    }
  }

  function handleDragOver(id: string, zone: DropZone) {
    dropTarget = { id, zone };
  }

  function handleDrop(id: string) {
    if (dragSourceId && dropTarget && domain.onDrop) {
      domain.onDrop(dragSourceId, dropTarget.id, dropTarget.zone);
    }
    dragSourceId = null;
    dropTarget = null;
  }

  function handleDragEnd() {
    dragSourceId = null;
    dropTarget = null;
  }

  // ─── Toggle ────────────────────────────────────────────────────────────

  function handleToggle(rowId: string, columnId: string, value: boolean, propagate: boolean) {
    domain.onToggle?.(rowId, columnId, value, propagate);
  }

  // ─── Filter input ──────────────────────────────────────────────────────

  function handleFilterKeydown(event: KeyboardEvent) {
    // Don't let the filter input's keydown bubble to the treelist keyboard handler
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.stopPropagation();
      // Transfer focus to the scroll container
      scrollEl?.focus();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="treelist"
  onkeydown={handleKeydown}
  ondragend={handleDragEnd}
  data-domain={domain.domainId}
>
  <!-- Filter bar -->
  <div class="ol-filter-bar">
    <input
      class="ol-filter-input"
      type="search"
      placeholder="Filter…"
      bind:value={filterString}
      onkeydown={handleFilterKeydown}
      aria-label="Filter rows"
    />
  </div>

  <!-- Scroll container -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="ol-scroll" bind:this={scrollEl} tabindex="0">
    {#each displayItems as item (item.id)}
      {#if item.kind === "group"}
        <TreeListGroup
          {item}
          {stateStore}
          columns={domain.columns}
          open={groupOpenState.get(item.id) ?? true}
          ontoggle={toggleGroup}
        />
      {:else}
        <TreeListRow
          {item}
          columns={domain.columns}
          selected={selectedId === item.id}
          active={activeId === item.id}
          editing={editingId === item.id}
          dragTarget={dropTarget?.id === item.id ? dropTarget.zone : null}
          onclick={handleRowClick}
          ontoggle={handleToggle}
          onrename={handleRename}
          onrenamecomplete={handleRenameComplete}
          oncontextmenu={handleContextMenu}
          ondragstart={handleDragStart}
          ondragover={handleDragOver}
          ondrop={handleDrop}
        />
      {/if}
    {/each}

    {#if overflowCount > 0}
      <div class="ol-overflow">
        ({overflowCount} more hidden)
      </div>
    {/if}
  </div>

  <!-- Context menu (portal-like: fixed position, renders above everything) -->
  {#if contextMenu}
    <ContextMenu
      items={contextMenu.items}
      x={contextMenu.x}
      y={contextMenu.y}
      onaction={handleContextAction}
      onclose={closeContextMenu}
    />
  {/if}
</div>

<style>
  .treelist {
    display: flex;
    flex-direction: column;
    height: 100%;
    container-type: inline-size;
    container-name: treelist;
  }

  /* ── Filter bar ─────────────────────────────────────────────────────────── */
  .ol-filter-bar {
    padding: 4px;
    border-bottom: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.06));
    flex-shrink: 0;
  }

  .ol-filter-input {
    width: 100%;
    height: 22px;
    padding: 0 6px;
    border: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.1));
    border-radius: 3px;
    background: var(--fill-inset, oklch(0.14 0.015 250));
    color: var(--text-mid, #ccc);
    font-size: 11px;
    font-family: inherit;
    outline: none;
  }

  .ol-filter-input:focus {
    border-color: var(--interactive-fill, oklch(0.55 0.15 250));
  }

  .ol-filter-input::placeholder {
    color: var(--text-faint, #555);
  }

  /* ── Scroll container ───────────────────────────────────────────────────── */
  /* Recessed well — same treatment as Sparkline/TimelineCanvas data areas. */
  .ol-scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 2px 4px;
    outline: none;
    background: var(--fill-inset, oklch(0 0 0 / 0.22));
    border-radius: 3px;
    box-shadow: var(--shadow-inset, inset 0 1px 3px oklch(0 0 0 / 0.30));
    margin: 0 0 4px;
  }

  /* ── Overflow indicator ─────────────────────────────────────────────────── */
  .ol-overflow {
    padding: 6px 0;
    text-align: center;
    font-size: 11px;
    color: var(--text-faint, #555);
    font-style: italic;
  }
</style>
