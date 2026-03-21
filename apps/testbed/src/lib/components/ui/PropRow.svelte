<script lang="ts">
  import { onDestroy } from "svelte";
  import { Copy, Check } from "lucide-svelte";

  let { label, value }: { label: string; value: string } = $props();

  let copied = $state(false);
  let hovering = $state(false);
  let timeoutId: ReturnType<typeof setTimeout>;

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      copied = true;
      clearTimeout(timeoutId);
      timeoutId = setTimeout(() => { copied = false; }, 1400);
    } catch {
      // clipboard unavailable (non-https, etc.)
    }
  }

  onDestroy(() => clearTimeout(timeoutId));
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="prop-row"
  onmouseenter={() => { hovering = true; }}
  onmouseleave={() => { hovering = false; }}
>
  <span class="prop-key">{label}</span>
  <div class="prop-right">
    <span class="prop-val">{value}</span>
    <button
      class="copy-btn"
      class:visible={hovering || copied}
      class:done={copied}
      onclick={copy}
      tabindex="-1"
      aria-label="Copy value"
    >
      {#if copied}
        <Check size={10} strokeWidth={2.5} />
      {:else}
        <Copy size={10} strokeWidth={1.8} />
      {/if}
    </button>
  </div>
</div>

<style>
  .prop-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    min-height: 20px;
  }

  .prop-key {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-subtle);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .prop-right {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
  }

  .prop-val {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-mid);
    text-align: right;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 130px;
  }

  .copy-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--text-faint);
    border-radius: 2px;
    padding: 0;
    opacity: 0;
    transition: opacity 0.1s ease, color 0.1s ease;
    flex-shrink: 0;
  }

  .copy-btn.visible {
    opacity: 1;
  }

  .copy-btn.done {
    color: var(--color-success);
  }

  .copy-btn:hover {
    color: var(--text-mid);
  }
</style>
