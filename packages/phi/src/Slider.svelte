<script lang="ts">
  /**
   * Slider — Simple labeled range input.
   *
   * USE WHEN: The user needs to explore a numeric range where precision isn't
   * critical — volume, opacity, a visualization parameter.
   *
   * PREFER INSTEAD:
   * - ScrubField — when fine control matters (drag scrub, step buttons, type-to-edit)
   *
   * Uses a native HTML range input. Value shown right-aligned in the header.
   */
  let {
    id,
    label,
    min,
    max,
    step,
    value,
    decimals = 2,
    onValueChange,
  }: {
    id: string;
    label: string;
    min: number;
    max: number;
    step: number;
    value: number;
    decimals?: number;
    onValueChange: (v: number) => void;
  } = $props();

  let current = $state(value);

  function handleInput(e: Event) {
    current = Number((e.currentTarget as HTMLInputElement).value);
    onValueChange(current);
  }
</script>

<div class="slider-field">
  <div class="slider-header">
    <label class="label" for={id}>{label}</label>
    <span class="prop-val">{current.toFixed(decimals)}</span>
  </div>
  <input
    {id}
    type="range"
    {min}
    {max}
    {step}
    value={current}
    oninput={handleInput}
  />
</div>

<style>
  .slider-field {
    padding: 4px 0;
  }

  .slider-header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 3px;
    font-size: 11px;
    font-weight: 500;
    color: var(--text-subtle);
  }

  .prop-val {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-mid);
  }
</style>
