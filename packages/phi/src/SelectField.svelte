<script lang="ts">
  /**
   * SelectField — Dropdown select for choosing one of many options.
   *
   * USE WHEN: The user needs to pick from 5+ options, or when horizontal
   * space is limited and a dropdown is more appropriate than always-visible buttons.
   *
   * PREFER INSTEAD:
   * - ToggleGroup — for 2-4 options that should always be visible (render mode, view type)
   *
   * PROPS: `options` is [{ value, label }]. `inline` variant uses smaller font/padding.
   * Built on bits-ui Select primitive for accessible dropdown behavior.
   */
  import { Select } from "bits-ui";
  import { ChevronDown, Check } from "lucide-svelte";

  let {
    id,
    options,
    value,
    inline = false,
    onValueChange,
  }: {
    id?: string;
    options: { value: string; label: string }[];
    value: string;
    inline?: boolean;
    onValueChange: (v: string) => void;
  } = $props();

  const selectedLabel = $derived(options.find((o) => o.value === value)?.label ?? value);
</script>

<Select.Root
  type="single"
  {value}
  {onValueChange}
  items={options.map((o) => ({ value: o.value, label: o.label }))}
>
  <Select.Trigger {id} class={`sel-trigger${inline ? " sel-inline" : ""}`} aria-label={selectedLabel}>
    <span class="sel-value">{selectedLabel}</span>
    <ChevronDown class="sel-chevron" size={12} />
  </Select.Trigger>

  <Select.Content class="sel-content" sideOffset={4}>
    <Select.Viewport class="sel-viewport">
      {#each options as opt (opt.value)}
        <Select.Item value={opt.value} label={opt.label} class="sel-item">
          {#snippet children({ selected })}
            <span class="sel-item-text">{opt.label}</span>
            {#if selected}
              <Check class="sel-item-check" size={11} />
            {/if}
          {/snippet}
        </Select.Item>
      {/each}
    </Select.Viewport>
  </Select.Content>
</Select.Root>

<style>
  /* ── Trigger ────────────────────────────────────────────────────── */
  :global(.sel-trigger) {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    width: 100%;
    padding: 5px 8px;
    font-size: 12px;
    font-weight: 500;
    font-family: var(--font-sans);
    color: var(--text-mid);
    background: var(--fill-lo);
    border: 1px solid var(--stroke-mid);
    border-radius: 4px;
    cursor: pointer;
    outline: none;
    transition: background 0.12s ease, border-color 0.12s ease;
    white-space: nowrap;
    user-select: none;
  }

  :global(.sel-trigger:hover) {
    background: var(--fill-mid);
    border-color: var(--stroke-hi);
  }

  :global(.sel-trigger[data-state="open"]) {
    background: var(--fill-mid);
    border-color: oklch(0.80 0.16 250 / 50%);
  }

  :global(.sel-trigger:focus-visible) {
    border-color: oklch(0.80 0.16 250 / 60%);
    box-shadow: 0 0 0 2px var(--interactive-ring);
  }

  :global(.sel-trigger.sel-inline) {
    width: auto;
    padding: 3px 7px;
    font-size: 11px;
  }

  /* ── Trigger internals ──────────────────────────────────────────── */
  :global(.sel-value) {
    flex: 1;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  :global(.sel-chevron) {
    flex-shrink: 0;
    color: var(--text-subtle);
    transition: transform 0.15s ease;
  }

  :global([data-state="open"] .sel-chevron) {
    transform: rotate(180deg);
  }

  /* ── Content popup ──────────────────────────────────────────────── */
  :global(.sel-content) {
    z-index: 9990;
    background: var(--surface-5);
    border: 1px solid var(--stroke-mid);
    border-radius: 4px;
    padding: 3px;
    box-shadow: 0 6px 24px oklch(0 0 0 / 40%);
    width: var(--bits-select-anchor-width);
    outline: none;
  }

  :global(.sel-content[data-state="open"]) {
    animation: sel-in 0.1s ease;
  }

  @keyframes sel-in {
    from { opacity: 0; transform: translateY(-4px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  :global(.sel-viewport) {
    max-height: 240px;
    overflow-y: auto;
  }

  /* ── Items ──────────────────────────────────────────────────────── */
  :global(.sel-item) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 5px 8px;
    font-size: 12px;
    font-weight: 400;
    font-family: var(--font-sans);
    color: var(--text-mid);
    border-radius: 3px;
    cursor: pointer;
    outline: none;
    gap: 8px;
    user-select: none;
  }

  :global(.sel-item[data-highlighted]) {
    background: var(--interactive-fill);
    color: var(--text-hi);
  }

  :global(.sel-item[data-selected]) {
    color: var(--text-hi);
  }

  :global(.sel-item-text) {
    flex: 1;
  }

  :global(.sel-item-check) {
    flex-shrink: 0;
    color: var(--interactive);
  }
</style>
