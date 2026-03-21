<script lang="ts">
  import { backendStore } from "$lib/stores/viewer";
  import { requestGpuDevice } from "$lib/utils/gpu";
  import Section from "$lib/components/ui/Section.svelte";
  import PropRow from "$lib/components/ui/PropRow.svelte";
  import ScrubField from "$lib/components/ui/ScrubField.svelte";
  import SelectField from "$lib/components/ui/SelectField.svelte";
  import CheckboxRow from "$lib/components/ui/CheckboxRow.svelte";

  const rendererOptions = [
    { value: "auto",   label: "Auto" },
    { value: "webgpu", label: "WebGPU" },
    { value: "webgl",  label: "WebGL2" },
  ];

  const rendererLabel = $derived(
    $backendStore ? ($backendStore.isWebGPU ? "WebGPU" : "WebGL2") : "—"
  );

  type GpuLimits = { invocations: string; storageMB: string } | null;
  let gpuLimits = $state<GpuLimits>(null);

  requestGpuDevice().then((device) => {
    if (!device) return;
    const l = device.limits;
    gpuLimits = {
      invocations: l.maxComputeInvocationsPerWorkgroup.toLocaleString(),
      storageMB: `${Math.round(l.maxStorageBufferBindingSize / (1024 * 1024))} MB`,
    };
  });

  const savedPref =
    (localStorage.getItem("rendererPreference") as "auto" | "webgpu" | "webgl") ?? "auto";

  function onRendererChange(value: string) {
    localStorage.setItem("rendererPreference", value);
    window.location.reload();
  }

  let lockResolution = $state(false);
  let lockedWidth = $state(960);
  let lockedHeight = $state(540);

  function applyResolution() {
    const canvas = document.querySelector<HTMLCanvasElement>("#viewport-canvas");
    if (!canvas || !$backendStore) return;
    if (lockResolution) {
      canvas.style.width = `${lockedWidth}px`;
      canvas.style.height = `${lockedHeight}px`;
      $backendStore.resize(lockedWidth, lockedHeight);
    } else {
      canvas.style.width = "100%";
      canvas.style.height = "100%";
      const rect = canvas.getBoundingClientRect();
      $backendStore.resize(rect.width, rect.height);
    }
  }
</script>

<div class="panel-content">
  <div class="section-header">Debug</div>

  <Section sectionId="debug-device" title="Device">
    <PropRow label="Renderer" value={rendererLabel} />
    {#if gpuLimits}
      <PropRow label="Max invocations" value={gpuLimits.invocations} />
      <PropRow label="Max storage" value={gpuLimits.storageMB} />
    {:else}
      <PropRow label="Limits" value="querying…" />
    {/if}

    <div class="field-row" style="margin-top: 8px;">
      <span class="label">Preference</span>
      <div style="width: 110px;">
        <SelectField
          options={rendererOptions}
          value={savedPref}
          inline
          onValueChange={onRendererChange}
        />
      </div>
    </div>
  </Section>

  <Section sectionId="debug-lighting" title="Lighting">
    {#if $backendStore}
      <ScrubField
        label="Exposure"
        value={$backendStore.getExposure()}
        defaultValue={1.0}
        min={0.6}
        max={2.5}
        step={0.05}
        decimals={2}
        onValueChange={(v) => $backendStore?.setExposure(v)}
      />
      <ScrubField
        label="Light Scale"
        value={$backendStore.getLightScale()}
        defaultValue={1.0}
        min={0.2}
        max={3.0}
        step={0.1}
        decimals={2}
        onValueChange={(v) => $backendStore?.setLightScale(v)}
      />
    {/if}
  </Section>

  <Section sectionId="debug-resolution" title="Resolution">
    <CheckboxRow
      label="Lock render size"
      checked={lockResolution}
      onchange={(v) => { lockResolution = v; applyResolution(); }}
    />

    {#if lockResolution}
      <div class="res-inputs">
        <input
          type="number"
          min="320"
          max="3840"
          value={lockedWidth}
          onchange={(e) => { lockedWidth = Number(e.currentTarget.value); applyResolution(); }}
        />
        <span class="value-muted">×</span>
        <input
          type="number"
          min="240"
          max="2160"
          value={lockedHeight}
          onchange={(e) => { lockedHeight = Number(e.currentTarget.value); applyResolution(); }}
        />
      </div>
    {/if}
  </Section>
</div>

<style>
  .field-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .res-inputs {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
  }

  .res-inputs input[type="number"] {
    width: 72px;
  }
</style>
