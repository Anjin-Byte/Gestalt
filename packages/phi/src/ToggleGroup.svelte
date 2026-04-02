<script lang="ts">
  /**
   * ToggleGroup — Segmented radio buttons for mutually exclusive selection.
   *
   * USE WHEN: The user chooses between 2-4 mutually exclusive options and
   * all options should be visible at once. Render mode (Solid/Wire/Normals/Depth),
   * view type, coordinate space — anywhere a small set of choices matters.
   *
   * PREFER INSTEAD:
   * - SelectField — for 5+ options or when horizontal space is tight
   * - CheckboxRow — for on/off toggles (not mutually exclusive choices)
   *
   * FEATURES: Keyboard arrow navigation with wrap, divider lines between segments,
   * highlighted selected state. 22px height fits dense panel layouts.
   */
  import { setHint, clearHint } from "$lib/stores/status";

  let {
    options,
    value,
    onValueChange,
    label,
  }: {
    options: { value: string; label: string }[];
    value: string;
    onValueChange: (v: string) => void;
    /** aria-label for the radiogroup — required for screen reader context. */
    label?: string;
  } = $props();

  let containerEl: HTMLDivElement | undefined = $state();

  function handleKeydown(e: KeyboardEvent, idx: number) {
    let next: number | null = null;

    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      next = (idx + 1) % options.length;
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      next = (idx - 1 + options.length) % options.length;
    }

    if (next !== null) {
      onValueChange(options[next].value);
      const btns = containerEl?.querySelectorAll<HTMLButtonElement>(".tg-btn");
      btns?.[next]?.focus();
    }
  }
</script>

<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
  bind:this={containerEl}
  class="tg-root"
  role="radiogroup"
  aria-label={label}
  onmouseenter={() => setHint("Click or use ← → arrow keys to change selection")}
  onmouseleave={() => clearHint()}
>
  {#each options as opt, i (opt.value)}
    <button
      class="tg-btn"
      class:selected={value === opt.value}
      role="radio"
      aria-checked={value === opt.value}
      tabindex={value === opt.value ? 0 : -1}
      onclick={() => onValueChange(opt.value)}
      onkeydown={(e) => handleKeydown(e, i)}
    >{opt.label}</button>
  {/each}
</div>

<style>
  /* ── Container ─────────────────────────────────────────────────────── */
  .tg-root {
    display: flex;
    height: 22px;
    border: 1px solid var(--stroke-mid);
    border-radius: 4px;
    overflow: hidden;
    transition: border-color 0.1s ease;
  }

  .tg-root:hover {
    border-color: var(--stroke-hi);
  }

  /* ── Buttons ───────────────────────────────────────────────────────── */
  .tg-btn {
    flex: 1;
    min-width: 0;
    background: var(--fill-lo);
    border: none;
    /* Right-side divider between adjacent segments.                      */
    /* Uses stroke-lo (dimmer than the outer border) so it recedes.       */
    border-right: 1px solid var(--stroke-lo);
    color: var(--text-subtle);
    font-size: 11px;
    font-weight: 500;
    font-family: var(--font-sans);
    cursor: pointer;
    padding: 0 6px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    outline: none;
    transition: background 0.1s ease, color 0.1s ease;
    /* Disable user selection so drag across segments doesn't highlight. */
    user-select: none;
  }

  .tg-btn:last-child {
    border-right: none;
  }

  .tg-btn:hover:not(.selected) {
    background: var(--fill-mid);
    color: var(--text-mid);
  }

  /* ── Selected state ────────────────────────────────────────────────── */
  /* Background tint at ~18% — clearly above hover (8% fill-mid) but     */
  /* not as heavy as a solid fill, keeping the segment feel lightweight.  */
  .tg-btn.selected {
    background: oklch(0.80 0.16 250 / 18%);
    color: var(--interactive);
  }

  .tg-btn.selected:hover {
    background: oklch(0.80 0.16 250 / 22%);
  }

  /* ── Focus ring ────────────────────────────────────────────────────── */
  /* Inset shadow so it's not clipped by the container's overflow:hidden. */
  .tg-btn:focus-visible {
    box-shadow: inset 0 0 0 2px var(--interactive-ring);
  }
</style>
