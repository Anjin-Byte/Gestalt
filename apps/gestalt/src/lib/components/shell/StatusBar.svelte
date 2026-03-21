<script lang="ts">
  import { statusHint } from "$lib/stores/status";
  import { fpsText } from "$lib/stores/viewer";

  // Show the hover hint when present, otherwise fall back to FPS.
  const display = $derived($statusHint || $fpsText);
</script>

<div class="status-bar">
  <span class="status-text">{display}</span>
</div>

<style>
  .status-bar {
    height: 22px;
    min-height: 22px;
    display: flex;
    align-items: center;
    padding: 0 12px;
    background: var(--surface-0);
    border-top: 1px solid var(--stroke-lo);
    flex-shrink: 0;
  }

  .status-text {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: color 0.1s ease;
  }

  /* Brighten when showing an active hint */
  .status-bar:has(.status-text:not(:empty)) .status-text {
    color: var(--text-lo);
  }
</style>
