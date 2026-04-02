<script lang="ts">
  import type { TreeListRowItem, TreeListColumnDef, DropZone } from "./types";
  import StatusIndicator from "../StatusIndicator.svelte";
  import Sparkline from "../Sparkline.svelte";

  let {
    item,
    columns,
    selected = false,
    active = false,
    editing = false,
    dragTarget = null,
    onclick,
    ontoggle,
    onrename,
    onrenamecomplete,
    oncontextmenu,
    ondragstart,
    ondragover,
    ondrop,
  }: {
    item: TreeListRowItem;
    columns: TreeListColumnDef[];
    selected?: boolean;
    active?: boolean;
    /** When true, the name column shows an editable input. Controlled by parent. */
    editing?: boolean;
    dragTarget?: DropZone | null;
    onclick?: (id: string, event: MouseEvent) => void;
    ontoggle?: (rowId: string, columnId: string, value: boolean, propagate: boolean) => void;
    /** Called when the user confirms a rename (Enter). */
    onrename?: (id: string, newLabel: string) => void;
    /** Called when rename mode ends (confirm or cancel). */
    onrenamecomplete?: () => void;
    oncontextmenu?: (id: string, event: MouseEvent) => void;
    ondragstart?: (id: string, event: DragEvent) => void;
    ondragover?: (id: string, zone: DropZone) => void;
    ondrop?: (id: string) => void;
  } = $props();

  // Build grid-template-columns: 1fr for name, then each column's fixed width.
  const gridTemplate = $derived(
    "1fr " + columns.map((c) => `${c.width}px`).join(" ")
  );

  function computeZone(event: DragEvent, el: HTMLElement): DropZone {
    const rect = el.getBoundingClientRect();
    const relY = event.clientY - rect.top;
    const h = rect.height;
    if (relY < h * 0.25) return "before";
    if (relY > h * 0.75) return "after";
    return "into";
  }

  function handleDragOver(event: DragEvent) {
    if (!item.draggable || !ondragover) return;
    event.preventDefault();
    ondragover(item.id, computeZone(event, event.currentTarget as HTMLElement));
  }

  function handleDrop(event: DragEvent) {
    if (!ondrop) return;
    event.preventDefault();
    ondrop(item.id);
  }

  // ─── Inline Rename ──────────────────────────────────────────────────────

  let renameValue = $state("");
  let renameInputEl = $state<HTMLInputElement | null>(null);

  // When editing starts, seed the input and auto-focus
  $effect(() => {
    if (editing && renameInputEl) {
      renameValue = item.label;
      // Defer focus to next microtask so the input is mounted
      requestAnimationFrame(() => {
        renameInputEl?.focus();
        renameInputEl?.select();
      });
    }
  });

  function confirmRename() {
    const trimmed = renameValue.trim();
    if (trimmed && trimmed !== item.label) {
      onrename?.(item.id, trimmed);
    }
    onrenamecomplete?.();
  }

  function cancelRename() {
    onrenamecomplete?.();
  }

  function handleRenameKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      confirmRename();
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      cancelRename();
    }
  }

  function handleRenameBlur() {
    // Commit on blur (same as Blender — clicking away confirms)
    confirmRename();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="treelist-row"
  class:selected
  class:active
  class:faded={item.faded}
  class:drag-before={dragTarget === "before"}
  class:drag-into={dragTarget === "into"}
  class:drag-after={dragTarget === "after"}
  style="--depth: {item.depth ?? 0}; grid-template-columns: {gridTemplate}"
  draggable={item.draggable ? "true" : undefined}
  onclick={(e) => onclick?.(item.id, e)}
  oncontextmenu={(e) => oncontextmenu?.(item.id, e)}
  ondragstart={(e) => ondragstart?.(item.id, e)}
  ondragover={handleDragOver}
  ondrop={handleDrop}
  data-row-id={item.id}
>
  <!-- Name column -->
  {#if editing}
    <input
      class="ol-name-input"
      type="text"
      bind:value={renameValue}
      bind:this={renameInputEl}
      onkeydown={handleRenameKeydown}
      onblur={handleRenameBlur}
      onclick={(e) => e.stopPropagation()}
      aria-label="Rename {item.label}"
    />
  {:else}
    <span class="ol-name" title={item.label}>
      {item.label}
    </span>
  {/if}

  <!-- Cell columns -->
  {#each item.cells as cell, i}
    {@const col = columns[i]}
    {#if cell.type === "status"}
      <span class="ol-cell" data-col={col?.id} data-hide-below={col?.hideBelow}>
        <StatusIndicator status={cell.status} label={cell.label} />
      </span>
    {:else if cell.type === "mono"}
      <span class="ol-cell ol-mono" data-col={col?.id} data-hide-below={col?.hideBelow} title={cell.value}>
        {cell.value}
      </span>
    {:else if cell.type === "spark"}
      <span class="ol-cell" data-col={col?.id} data-hide-below={col?.hideBelow}>
        <Sparkline values={cell.values} height={14} warn={cell.warn} danger={cell.danger} />
      </span>
    {:else if cell.type === "toggle"}
      <span class="ol-cell" data-col={col?.id} data-hide-below={col?.hideBelow}>
        <button
          class="ol-toggle"
          class:ol-toggle-on={cell.value}
          class:ol-toggle-off={!cell.value}
          disabled={cell.disabled}
          aria-label={col?.label}
          aria-pressed={cell.value}
          onclick={(e) => {
            e.stopPropagation();
            ontoggle?.(item.id, col?.id ?? "", !cell.value, e.shiftKey);
          }}
        >
          {#if typeof cell.icon === "string"}
            {cell.icon}
          {:else}
            <svelte:component this={cell.icon} size={12} />
          {/if}
        </button>
      </span>
    {/if}
  {/each}
</div>

<style>
  .treelist-row {
    display: grid;
    align-items: center;
    height: 22px;
    padding-left: calc(var(--depth, 0) * 12px);
    border-radius: 3px;
    cursor: pointer;
    user-select: none;
    gap: 2px;
  }

  /* ── Selection states — matches Blender TH_SELECT_ACTIVE / TH_SELECT_HIGHLIGHT ── */
  /* Uses system alpha tokens so contrast works on any parent surface (flat or card). */
  .treelist-row:hover                 { background: var(--fill-mid, oklch(1 0 0 / 0.08)); }
  .treelist-row.selected              { background: var(--interactive-fill, oklch(0.80 0.16 250 / 0.14)); }
  .treelist-row.selected.active       { background: var(--interactive-fill, oklch(0.80 0.16 250 / 0.14));
                                        outline: 1px solid var(--interactive-ring, oklch(0.80 0.16 250 / 0.30)); }
  /* Active-only (not selected): no background — visually identical to unselected. */

  /* ── Drag target zones ──────────────────────────────────────────────────────── */
  .treelist-row.drag-into    { background: var(--fill-mid, oklch(1 0 0 / 0.08));
                               outline: 1px solid var(--stroke-strong, oklch(1 0 0 / 0.35)); }
  .treelist-row.drag-before  { border-top: 2px solid var(--stroke-strong, oklch(1 0 0 / 0.35)); }
  .treelist-row.drag-after   { border-bottom: 2px solid var(--stroke-strong, oklch(1 0 0 / 0.35)); }

  /* ── Faded row ──────────────────────────────────────────────────────────────── */
  .treelist-row.faded .ol-name { opacity: 0.5; }

  /* ── Name column ────────────────────────────────────────────────────────────── */
  .ol-name {
    font-size: 12px;
    font-weight: 400;
    color: var(--text-mid, #ccc);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    padding-left: 4px;
  }

  /* ── Rename input ────────────────────────────────────────────────────────────── */
  .ol-name-input {
    font-size: 12px;
    font-weight: 400;
    font-family: inherit;
    color: var(--text-mid, #ccc);
    background: var(--fill-inset, oklch(0.14 0.015 250));
    border: 1px solid var(--interactive-fill, oklch(0.55 0.15 250));
    border-radius: 2px;
    padding: 0 4px;
    height: 18px;
    width: 100%;
    min-width: 0;
    outline: none;
    box-sizing: border-box;
  }

  /* ── Cell columns ───────────────────────────────────────────────────────────── */
  .ol-cell {
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }

  .ol-mono {
    font-family: var(--font-mono, monospace);
    font-size: 11px;
    color: var(--text-subtle, #999);
    white-space: nowrap;
    text-overflow: ellipsis;
    justify-content: flex-end;
    padding-right: 2px;
  }

  /* ── Toggle button ──────────────────────────────────────────────────────────── */
  .ol-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    border-radius: 2px;
    background: none;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    color: var(--text-subtle, #999);
    transition: color 0.1s ease, background 0.1s ease;
  }

  .ol-toggle:hover:not(:disabled) {
    background: var(--fill-mid, oklch(1 0 0 / 0.08));
    color: var(--text-mid, #ccc);
  }

  .ol-toggle:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .ol-toggle-on {
    color: var(--text-mid, #ccc);
  }

  .ol-toggle-off {
    color: var(--text-faint, #555);
  }
</style>
