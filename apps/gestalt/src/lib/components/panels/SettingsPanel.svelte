<script lang="ts">
  import { backendStore } from "$lib/stores/viewer";
  import { Section, SelectField, CheckboxRow, ScrubField } from "@gestalt/phi";

  const rendererOptions = [
    { value: "auto",   label: "Auto"   },
    { value: "webgpu", label: "WebGPU" },
    { value: "webgl",  label: "WebGL2" },
  ];

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
  <div class="section-header">Settings</div>

  <Section sectionId="settings-renderer" title="Renderer">
    <div class="field-row">
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

  <Section sectionId="settings-resolution" title="Resolution">
    <CheckboxRow
      label="Lock render size"
      checked={lockResolution}
      onchange={(v) => { lockResolution = v; applyResolution(); }}
    />

    {#if lockResolution}
      <div class="res-row" style="margin-top: 6px;">
        <ScrubField
          label="W"
          value={lockedWidth}
          min={320}
          max={3840}
          step={1}
          decimals={0}
          onValueChange={(v) => { lockedWidth = v; applyResolution(); }}
        />
        <ScrubField
          label="H"
          value={lockedHeight}
          min={240}
          max={2160}
          step={1}
          decimals={0}
          onValueChange={(v) => { lockedHeight = v; applyResolution(); }}
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

  .res-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
</style>
