<script lang="ts">
  /**
   * DropOverlay — Visual indicator showing where a dragged panel will land.
   *
   * Rendered inside a DockGroup when a drag is active over that group.
   * Shows 5 zones: center (tabify) + 4 edges (split). The active zone
   * is highlighted with a semi-transparent fill.
   *
   * GPU-composited via will-change: transform. 70ms CSS transition for
   * smooth zone switching during drag movement.
   *
   * See: reference/dock-implementation-guide.md — Drop Zone Overlay Rendering
   */

  import type { DropZonePosition } from "./dnd";

  let {
    zone,
  }: {
    /** The currently active drop zone, or null if no zone is active. */
    zone: DropZonePosition | null;
  } = $props();
</script>

<div class="dock-drop-overlay">
  <!-- Center zone: full area, shown when tabifying -->
  <div
    class="dock-drop-zone dock-drop-center"
    class:active={zone === "center"}
  ></div>

  <!-- Edge zones: half-area indicators for split directions -->
  <div
    class="dock-drop-zone dock-drop-left"
    class:active={zone === "left"}
  ></div>
  <div
    class="dock-drop-zone dock-drop-right"
    class:active={zone === "right"}
  ></div>
  <div
    class="dock-drop-zone dock-drop-top"
    class:active={zone === "top"}
  ></div>
  <div
    class="dock-drop-zone dock-drop-bottom"
    class:active={zone === "bottom"}
  ></div>
</div>

<style>
  .dock-drop-overlay {
    position: absolute;
    inset: 0;
    z-index: 100;
    pointer-events: none;
    will-change: transform;
    transform: translate3d(0, 0, 0);
  }

  .dock-drop-zone {
    position: absolute;
    background: oklch(0.65 0.15 250 / 0);
    border: 2px solid oklch(0.65 0.15 250 / 0);
    border-radius: 4px;
    transition: background 70ms ease, border-color 70ms ease;
  }

  .dock-drop-zone.active {
    background: oklch(0.65 0.15 250 / 0.15);
    border-color: oklch(0.65 0.15 250 / 0.5);
  }

  /* Center zone: full area */
  .dock-drop-center {
    inset: 4px;
  }

  /* Edge zones: 50% of the area on the corresponding side */
  .dock-drop-left {
    top: 4px;
    left: 4px;
    bottom: 4px;
    width: calc(50% - 6px);
  }

  .dock-drop-right {
    top: 4px;
    right: 4px;
    bottom: 4px;
    width: calc(50% - 6px);
  }

  .dock-drop-top {
    top: 4px;
    left: 4px;
    right: 4px;
    height: calc(50% - 6px);
  }

  .dock-drop-bottom {
    bottom: 4px;
    left: 4px;
    right: 4px;
    height: calc(50% - 6px);
  }
</style>
