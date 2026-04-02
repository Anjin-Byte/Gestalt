<script lang="ts">
  /**
   * ActionButton — Styled button for one-shot actions.
   *
   * USE WHEN: The user needs to trigger an action — reset view, pause timeline,
   * export data, frame selected object. Not for toggling state (use CheckboxRow)
   * or choosing options (use ToggleGroup/SelectField).
   *
   * FEATURES: Optional icon via snippet slot, full-width variant, disabled state.
   */
  import type { Snippet } from "svelte";

  let {
    children,
    onclick,
    disabled = false,
    fullWidth = false,
  }: {
    children: Snippet;
    onclick?: () => void;
    disabled?: boolean;
    fullWidth?: boolean;
  } = $props();
</script>

<button
  class="action-btn"
  class:full-width={fullWidth}
  {disabled}
  {onclick}
>
  {@render children()}
</button>

<style>
  .action-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 5px 12px;
    min-height: 26px;
    font-size: 12px;
    font-weight: 500;
    font-family: var(--font-sans);
    color: var(--text-hi);
    background: oklch(0.32 0.03 250);
    border: 1px solid var(--stroke-mid);
    border-radius: 4px;
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
    transition: background 0.1s ease, border-color 0.1s ease;
  }

  .action-btn:hover:not(:disabled) {
    background: oklch(0.38 0.04 250);
    border-color: var(--stroke-hi);
  }

  .action-btn:active:not(:disabled) {
    background: oklch(0.28 0.02 250);
  }

  .action-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .action-btn:focus-visible {
    outline: none;
    border-color: oklch(0.80 0.16 250 / 60%);
    box-shadow: 0 0 0 2px var(--interactive-ring);
  }

  .action-btn.full-width {
    width: 100%;
  }
</style>
