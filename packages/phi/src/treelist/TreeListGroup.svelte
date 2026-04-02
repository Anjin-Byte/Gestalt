<script lang="ts">
  import type { TreeListGroupItem, TreeListColumnDef } from "./types";
  import { TreeListStateStore } from "./types";
  import { ChevronRight } from "lucide-svelte";
  import BarMeter from "../BarMeter.svelte";

  let {
    item,
    stateStore,
    columns,
    open = undefined,
    ontoggle,
  }: {
    item: TreeListGroupItem;
    stateStore: TreeListStateStore;
    columns: TreeListColumnDef[];
    /** Override open state (used when parent controls reactivity). */
    open?: boolean;
    /** Called when the group header is clicked. Parent handles state mutation. */
    ontoggle?: (groupId: string) => void;
  } = $props();

  // If open is provided externally, use it. Otherwise derive from stateStore.
  const isOpen = $derived(open ?? !stateStore.isCollapsed(item.id));

  // Same grid as TreeListRow so columns align.
  const gridTemplate = $derived(
    "1fr " + columns.map((c) => `${c.width}px`).join(" ")
  );

  function toggle() {
    if (ontoggle) {
      ontoggle(item.id);
    } else {
      stateStore.toggle(item.id);
    }
  }
</script>

<button
  class="treelist-group"
  style="grid-template-columns: {gridTemplate}"
  onclick={toggle}
  aria-expanded={isOpen}
  data-group-id={item.id}
>
  <!-- Name track: chevron + label + optional count -->
  <span class="og-name">
    <span class="og-chevron" class:open={isOpen}>
      <ChevronRight size={11} strokeWidth={2} />
    </span>
    <span class="og-label">{item.label}</span>
    {#if item.count != null}
      <span class="og-count">{item.count}</span>
    {/if}
  </span>

  <!-- Column tracks — only the last track shows aggregate if present -->
  {#each columns as col, i}
    <span class="og-col" data-col={col.id} data-hide-below={col.hideBelow}>
      {#if i === columns.length - 1 && item.aggregate}
        <BarMeter
          label=""
          value={item.aggregate.value}
          max={item.aggregate.max}
          unit={item.aggregate.unit}
        />
      {/if}
    </span>
  {/each}
</button>

<style>
  .treelist-group {
    display: grid;
    align-items: center;
    height: 24px;
    gap: 2px;
    width: 100%;
    padding: 0;
    background: none;
    border: none;
    border-bottom: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.06));
    cursor: pointer;
    user-select: none;
    text-align: left;
  }

  .treelist-group:hover {
    background: var(--fill-lo, oklch(1 0 0 / 0.05));
  }

  /* ── Name track ─────────────────────────────────────────────────────────── */
  .og-name {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-left: 2px;
    overflow: hidden;
  }

  .og-chevron {
    display: flex;
    align-items: center;
    color: var(--text-faint, #555);
    transition: transform 0.14s ease, color 0.1s ease;
    flex-shrink: 0;
  }

  .og-chevron.open {
    transform: rotate(90deg);
    color: var(--text-subtle, #999);
  }

  .og-label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-lo, #777);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .og-count {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-faint, #555);
    background: var(--fill-lo, oklch(1 0 0 / 0.05));
    padding: 0 5px;
    border-radius: 8px;
    line-height: 16px;
    flex-shrink: 0;
  }

  /* ── Column tracks ──────────────────────────────────────────────────────── */
  .og-col {
    display: flex;
    align-items: center;
    overflow: hidden;
  }
</style>
