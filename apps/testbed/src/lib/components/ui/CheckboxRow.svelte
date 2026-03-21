<script lang="ts">
  let {
    label,
    checked = false,
    disabled = false,
    onchange,
  }: {
    label: string;
    checked?: boolean;
    disabled?: boolean;
    onchange: (checked: boolean) => void;
  } = $props();
</script>

<label class="cb-row" class:disabled>
  <input
    class="cb-input"
    type="checkbox"
    {checked}
    {disabled}
    onchange={(e) => onchange(e.currentTarget.checked)}
  />
  <span class="cb-box" aria-hidden="true"></span>
  <span class="cb-label">{label}</span>
</label>

<style>
  .cb-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 22px;
    cursor: pointer;
    user-select: none;
  }

  .cb-row.disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Visually hidden but accessible */
  .cb-input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
    pointer-events: none;
  }

  /* Custom box */
  .cb-box {
    position: relative;
    flex-shrink: 0;
    width: 13px;
    height: 13px;
    border: 1px solid var(--stroke-mid);
    border-radius: 2px;
    background: var(--fill-lo);
    transition: background 0.1s ease, border-color 0.1s ease;
  }

  /* Checkmark */
  .cb-box::after {
    content: '';
    position: absolute;
    left: 3.5px;
    top: 1px;
    width: 4px;
    height: 7px;
    border: 1.5px solid white;
    border-top: none;
    border-left: none;
    transform: rotate(45deg);
    opacity: 0;
    transition: opacity 0.08s ease;
  }

  /* Hover — only border, not background, so checked state wins cleanly */
  .cb-row:not(.disabled):hover .cb-box {
    border-color: var(--stroke-hi);
  }

  /* Checked — placed after hover so same-specificity cascade wins */
  .cb-row .cb-input:checked ~ .cb-box {
    background: var(--interactive);
    border-color: var(--interactive);
  }

  .cb-input:checked ~ .cb-box::after {
    opacity: 1;
  }

  /* Focus ring */
  .cb-input:focus-visible ~ .cb-box {
    box-shadow: 0 0 0 2px var(--interactive-ring);
  }

  .cb-label {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-mid);
    line-height: 1;
  }
</style>
