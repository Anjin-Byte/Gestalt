<script lang="ts">
  import type { ModuleHost } from "@gestalt/modules";
  import { moduleControls, moduleValues, scheduleRun } from "$lib/stores/moduleControls";
  import ScrubField from "$lib/components/ui/ScrubField.svelte";
  import SelectField from "$lib/components/ui/SelectField.svelte";
  import CheckboxRow from "$lib/components/ui/CheckboxRow.svelte";
  import ActionButton from "$lib/components/ui/ActionButton.svelte";

  let { host }: { host: ModuleHost | null } = $props();

  const modules = $derived(host?.list() ?? []);
  let selectedId = $state("");

  $effect(() => {
    if (modules.length > 0 && !selectedId) {
      selectedId = modules[0].id;
    }
  });

  async function activateModule(id: string) {
    selectedId = id;
    await host?.activate(id);
  }

  function onValueChange(id: string, value: unknown) {
    moduleValues.update(v => ({ ...v, [id]: value }));
    $scheduleRun();
  }
</script>

<div class="panel-content">

  <div class="settings-section">
    <div class="label" style="margin-bottom: 6px;">Active Module</div>
    <SelectField
      options={modules.map((m) => ({ value: m.id, label: m.name }))}
      value={selectedId}
      onValueChange={activateModule}
    />

    <div style="margin-top: 8px;">
      <ActionButton fullWidth onclick={() => host?.runActive()}>Run Module</ActionButton>
    </div>
  </div>

  {#if $moduleControls.length > 0}
    <div class="settings-section">
      <div class="label">Module Controls</div>

      {#each $moduleControls as control (control.kind === "button" ? control.label : control.id)}
        <div class="control-row">

          {#if control.kind === "slider"}
            <ScrubField
              label={control.label}
              value={($moduleValues[control.id] as number) ?? control.initial}
              defaultValue={control.initial}
              min={control.min}
              max={control.max}
              step={control.step}
              onValueChange={(v) => onValueChange(control.id, v)}
            />

          {:else if control.kind === "number"}
            <ScrubField
              label={control.label}
              value={($moduleValues[control.id] as number) ?? control.initial}
              defaultValue={control.initial}
              min={control.min}
              max={control.max}
              step={control.step ?? 1}
              decimals={Number.isInteger(control.step ?? 1) ? 0 : 2}
              onValueChange={(v) => onValueChange(control.id, v)}
            />

          {:else if control.kind === "checkbox"}
            <CheckboxRow
              label={control.label}
              checked={($moduleValues[control.id] as boolean) ?? control.initial}
              onchange={(v) => onValueChange(control.id, v)}
            />

          {:else if control.kind === "select"}
            <div class="label" style="margin-bottom: 4px;">{control.label}</div>
            <SelectField
              options={control.options.map((o) => ({ value: o, label: o }))}
              value={($moduleValues[control.id] as string) ?? control.initial}
              onValueChange={(v) => onValueChange(control.id, v)}
            />

          {:else if control.kind === "text"}
            <div class="label">{control.label}</div>
            <div class="meta">{control.initial}</div>

          {:else if control.kind === "file"}
            <label class="label" for={control.id}>{control.label}</label>
            <input
              id={control.id}
              type="file"
              accept={control.accept}
              style="width: 100%; font-size: 12px; color: var(--muted-foreground);"
              onchange={(e) => {
                const file = e.currentTarget.files?.[0] ?? null;
                void control.onFile(file);
                $scheduleRun();
              }}
            />

          {:else if control.kind === "button"}
            <ActionButton fullWidth onclick={control.onClick}>{control.label}</ActionButton>
          {/if}

        </div>
      {/each}
    </div>
  {/if}

</div>

<style>
  .control-row {
    padding: 6px 0;
    border-bottom: 1px solid var(--stroke-lo);
  }

  .control-row:last-child {
    border-bottom: none;
  }


</style>
