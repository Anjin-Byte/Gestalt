<script lang="ts">
  import { viewerStore } from "$lib/stores/viewer";
  import Section from "$lib/components/ui/Section.svelte";
  import CheckboxRow from "$lib/components/ui/CheckboxRow.svelte";
  import ActionButton from "$lib/components/ui/ActionButton.svelte";
</script>

<div class="panel-content">
  <div class="section-header">Scene</div>

  <Section sectionId="scene-visualization" title="Visualization">
    <CheckboxRow label="Wireframe" onchange={(v) => $viewerStore?.setWireframe(v)} />
    <CheckboxRow label="Axes" checked={true} onchange={(v) => $viewerStore?.setAxesVisible(v)} />
    <CheckboxRow label="Grid" onchange={(v) => $viewerStore?.setGridVisible(v)} />
    <CheckboxRow label="Bounds" onchange={(v) => $viewerStore?.setBoundsVisible(v)} />
    <CheckboxRow label="Unlit" onchange={(v) => $viewerStore?.setUnlit(v)} />
  </Section>

  <Section sectionId="scene-camera" title="Camera">
    <div style="margin-top: 2px;">
      <ActionButton fullWidth onclick={() => $viewerStore?.frameObject()}>Frame Object</ActionButton>
    </div>
  </Section>
</div>
