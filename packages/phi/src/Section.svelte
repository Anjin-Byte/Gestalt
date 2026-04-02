<script lang="ts">
  /**
   * Section — Collapsible container with persistent state.
   *
   * USE WHEN: Grouping related controls or display rows in a panel.
   * Every panel is built from Sections. Collapse state is saved to
   * localStorage keyed by `sectionId`, so the user's layout persists.
   *
   * FEATURES: Chevron toggle, slide animation (140ms), optional `card`
   * variant with rounded border container. Uppercase title, 11px font.
   */
  import type { Snippet } from "svelte";
  import { slide } from "svelte/transition";
  import { ChevronRight } from "lucide-svelte";

  let {
    sectionId,
    title,
    children,
    card = false,
  }: {
    sectionId: string;
    title: string;
    children: Snippet;
    /** Card variant — rounded border container instead of bottom-separator. */
    card?: boolean;
  } = $props();

  const storageKey = `panel-section:${sectionId}`;
  let open = $state(localStorage.getItem(storageKey) !== "false");

  function toggle() {
    open = !open;
    localStorage.setItem(storageKey, String(open));
  }
</script>

<div class="section" class:card>
  <button class="section-trigger" onclick={toggle} aria-expanded={open}>
    <span class="chevron" class:open>
      <ChevronRight size={11} strokeWidth={2} />
    </span>
    <span class="section-title">{title}</span>
  </button>

  {#if open}
    <div class="section-body" transition:slide={{ duration: 140 }}>
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .section {
    border-bottom: 1px solid var(--stroke-lo);
  }

  .section:last-child {
    border-bottom: none;
  }

  /* ── Card variant ───────────────────────────────────────────────────────── */
  /* One step above parent surface (--fill-lo = white 5%).
     Border at --stroke-lo to separate without competing with content. */
  .section.card {
    background: var(--fill-lo, oklch(1 0 0 / 0.05));
    border: 1px solid var(--stroke-lo, oklch(1 0 0 / 0.06));
    border-radius: 6px;
    padding: 0 8px;
    margin-bottom: 6px;
  }

  .section.card:last-child {
    margin-bottom: 0;
  }

  .section-trigger {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    padding: 9px 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-lo);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    text-align: left;
    transition: color 0.1s ease;
  }

  .section-trigger:hover {
    color: var(--text-mid);
  }

  .chevron {
    display: flex;
    align-items: center;
    color: var(--text-faint);
    transition: transform 0.14s ease, color 0.1s ease;
    flex-shrink: 0;
  }

  .chevron.open {
    transform: rotate(90deg);
    color: var(--text-subtle);
  }

  .section-body {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 2px 0 10px;
    overflow: visible;
  }
</style>
