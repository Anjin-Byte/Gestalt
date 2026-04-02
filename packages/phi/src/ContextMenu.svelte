<script lang="ts">
  /**
   * ContextMenu — Fixed-position floating menu for contextual actions.
   *
   * USE WHEN: Providing right-click actions on a specific item — rename,
   * delete, duplicate, copy, paste. Supports keyboard navigation (arrows,
   * Enter, Escape), disabled items, danger styling, separator lines, and
   * shortcut hint text.
   *
   * PREFER INSTEAD:
   * - ToggleGroup/SelectField — for persistent choices (not contextual actions)
   * - ActionButton — for always-visible one-shot actions
   *
   * FEATURES: Viewport overflow clamping, click-outside dismissal, z-index 1000.
   */
  import { onMount } from "svelte";

  export interface ContextMenuItem {
    /** Unique action ID passed to onaction. */
    id: string;
    label: string;
    /** Keyboard shortcut hint shown right-aligned (e.g. "F2", "Del", "⌘D"). */
    shortcut?: string;
    /** Disabled items are shown grayed and cannot be clicked. */
    disabled?: boolean;
    /** Danger items are shown in destructive color (red). */
    danger?: boolean;
    /** Visual separator line before this item. */
    separator?: boolean;
  }

  let {
    items,
    x,
    y,
    onaction,
    onclose,
  }: {
    items: ContextMenuItem[];
    /** Cursor X position (px from viewport left). */
    x: number;
    /** Cursor Y position (px from viewport top). */
    y: number;
    /** Called when a menu item is selected. */
    onaction: (actionId: string) => void;
    /** Called when the menu should close (click outside, Escape, item selected). */
    onclose: () => void;
  } = $props();

  let menuEl = $state<HTMLElement | null>(null);
  let focusIdx = $state(-1);

  // ─── Positioning ─────────────────────────────────────────────────────────
  // Clamp so the menu doesn't overflow the viewport.

  let posX = $state(x);
  let posY = $state(y);

  onMount(() => {
    if (!menuEl) return;
    const rect = menuEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    posX = x + rect.width > vw ? Math.max(0, vw - rect.width - 4) : x;
    posY = y + rect.height > vh ? Math.max(0, vh - rect.height - 4) : y;

    // Focus the menu for keyboard navigation
    menuEl.focus();
  });

  // ─── Enabled items for keyboard navigation ──────────────────────────────

  const enabledIndices = $derived(
    items.reduce<number[]>((acc, item, i) => {
      if (!item.disabled) acc.push(i);
      return acc;
    }, [])
  );

  // ─── Handlers ────────────────────────────────────────────────────────────

  function handleKeydown(event: KeyboardEvent) {
    switch (event.key) {
      case "Escape": {
        event.preventDefault();
        event.stopPropagation();
        onclose();
        break;
      }
      case "ArrowDown": {
        event.preventDefault();
        const currentPos = enabledIndices.indexOf(focusIdx);
        const nextPos = currentPos < enabledIndices.length - 1 ? currentPos + 1 : 0;
        focusIdx = enabledIndices[nextPos] ?? -1;
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        const currentPos = enabledIndices.indexOf(focusIdx);
        const prevPos = currentPos > 0 ? currentPos - 1 : enabledIndices.length - 1;
        focusIdx = enabledIndices[prevPos] ?? -1;
        break;
      }
      case "Enter": {
        event.preventDefault();
        event.stopPropagation();
        if (focusIdx >= 0 && !items[focusIdx].disabled) {
          onaction(items[focusIdx].id);
          onclose();
        }
        break;
      }
    }
  }

  function handleItemClick(item: ContextMenuItem) {
    if (item.disabled) return;
    onaction(item.id);
    onclose();
  }

  function handleBackdropClick() {
    onclose();
  }
</script>

<!-- Invisible backdrop to catch clicks outside the menu -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="ctx-backdrop" onclick={handleBackdropClick} oncontextmenu={(e) => { e.preventDefault(); handleBackdropClick(); }}></div>

<!-- The menu itself -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="ctx-menu"
  bind:this={menuEl}
  style="left: {posX}px; top: {posY}px"
  onkeydown={handleKeydown}
  tabindex="0"
  role="menu"
>
  {#each items as item, i}
    {#if item.separator}
      <div class="ctx-separator"></div>
    {/if}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="ctx-item"
      class:ctx-disabled={item.disabled}
      class:ctx-danger={item.danger}
      class:ctx-focused={focusIdx === i}
      role="menuitem"
      aria-disabled={item.disabled}
      onclick={() => handleItemClick(item)}
      onpointerenter={() => { if (!item.disabled) focusIdx = i; }}
    >
      <span class="ctx-label">{item.label}</span>
      {#if item.shortcut}
        <span class="ctx-shortcut">{item.shortcut}</span>
      {/if}
    </div>
  {/each}
</div>

<style>
  /* ── Backdrop ───────────────────────────────────────────────────────────── */
  .ctx-backdrop {
    position: fixed;
    inset: 0;
    z-index: 999;
  }

  /* ── Menu ────────────────────────────────────────────────────────────────── */
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    min-width: 160px;
    max-width: 280px;
    padding: 4px 0;
    background: var(--fill-surface, oklch(0.18 0.015 250));
    border: 1px solid var(--stroke-mid, oklch(1 0 0 / 0.12));
    border-radius: 6px;
    box-shadow:
      0 4px 16px oklch(0 0 0 / 0.4),
      0 1px 3px oklch(0 0 0 / 0.2);
    outline: none;
    font-size: 12px;
  }

  /* ── Separator ──────────────────────────────────────────────────────────── */
  .ctx-separator {
    height: 1px;
    margin: 4px 8px;
    background: var(--stroke-lo, oklch(1 0 0 / 0.06));
  }

  /* ── Item ────────────────────────────────────────────────────────────────── */
  .ctx-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 5px 12px;
    cursor: pointer;
    user-select: none;
    border-radius: 3px;
    margin: 0 4px;
    color: var(--text-mid, #ccc);
    transition: background 0.06s ease;
  }

  .ctx-item.ctx-focused {
    background: var(--interactive-fill, oklch(0.80 0.16 250 / 0.14));
  }

  .ctx-item.ctx-disabled {
    color: var(--text-faint, #555);
    cursor: not-allowed;
  }

  .ctx-item.ctx-disabled.ctx-focused {
    background: none;
  }

  .ctx-item.ctx-danger {
    color: var(--color-destructive, oklch(0.68 0.18 25));
  }

  .ctx-item.ctx-danger.ctx-focused {
    background: oklch(0.68 0.18 25 / 0.15);
  }

  /* ── Label + shortcut ───────────────────────────────────────────────────── */
  .ctx-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ctx-shortcut {
    font-size: 10px;
    color: var(--text-faint, #555);
    white-space: nowrap;
    flex-shrink: 0;
  }
</style>
