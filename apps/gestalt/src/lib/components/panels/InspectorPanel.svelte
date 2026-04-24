<script lang="ts">
  import {
    Section, ToggleGroup, SelectField, StatusIndicator, PropRow,
    BarMeter, DiffRow, ScrubField, CheckboxRow, CounterRow,
    BitField, ActionButton, Slider,
  } from "@gestalt/phi";
  import type { BitFieldFlag } from "@gestalt/phi";
  import { rendererStatsStore } from "$lib/stores/rendererStats";
  import { frameTimeline } from "$lib/stores/timeline";
  import type { FrameSample } from "$lib/stores/timeline";
  import * as RC from "../../../renderer/RendererController";
  import { orbitReset } from "$lib/stores/orbitReset";

  // ─── Model loading ─────────────────────────────────────────────────────

  let fileInputEl: HTMLInputElement = undefined!;
  let voxelResolution = $state(62);
  let loadingModel = $state(false);
  let loadError = $state("");

  function triggerFileInput() {
    fileInputEl?.click();
  }

  async function handleFileSelect(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    loadError = "";
    loadingModel = true;
    try {
      const text = await file.text();
      const info = RC.loadModel(text, voxelResolution);
      orbitReset.set({ center: info.center, extent: info.extent });
    } catch (err: any) {
      loadError = err?.message ?? String(err);
      console.error("[InspectorPanel] OBJ load error:", err);
    } finally {
      loadingModel = false;
      input.value = ""; // reset so re-selecting same file works
    }
  }

  // ─── CPU mesh toggle ───────────────────────────────────────────────────

  let cpuMeshEnabled = $state(false);
  let freezeCullEnabled = $state(false);
  let frustumCullEnabled = $state(true);
  let hizCullEnabled = $state(true);
  let giEnabled = $state(false);
  let giBackendMode = $state("2"); // default: v3 world-space

  const giBackendOptions = [
    { value: "0", label: "Off" },
    { value: "1", label: "V2 Legacy" },
    { value: "2", label: "V3 World-Space" },
  ];

  function onGiBackendChange(v: string) {
    giBackendMode = v;
    RC.setGiBackend(parseInt(v, 10));
    giEnabled = v !== "0";
  }

  // ─── Render mode (ToggleGroup) ────────────────────────────────────────

  const modeOptions = [
    { value: "0",  label: "Solid" },
    { value: "2",  label: "Wire" },
    { value: "4",  label: "Normals" },
    { value: "16", label: "Depth" },
    { value: "32", label: "GI Atlas" },
    { value: "33", label: "GI Hits" },
    { value: "34", label: "GI Only" },
    { value: "35", label: "GI Atlas B" },
    { value: "36", label: "GI Normals" },
    { value: "37", label: "World Pos" },
    { value: "38", label: "Raw Texel" },
    { value: "39", label: "Single Dir" },
  ];

  let currentMode = $state("0");

  function onModeChange(v: string) {
    currentMode = v;
    RC.setRenderMode(parseInt(v, 10));
  }

  // ─── Stats ────────────────────────────────────────────────────────────

  const stats = $derived($rendererStatsStore);

  // Track previous mesh stats for DiffRow deltas
  let prevVerts = $state(0);
  let prevIndices = $state(0);
  $effect(() => {
    if (stats && stats.meshVerts !== prevVerts) prevVerts = stats.meshVerts;
    if (stats && stats.meshIndices !== prevIndices) prevIndices = stats.meshIndices;
  });

  // ─── FPS counter (CounterRow with sparkline) ──────────────────────────

  let fpsHistory = $state<number[]>([]);
  let currentFps = $state("—");

  $effect(() => {
    return frameTimeline.subscribe((history: FrameSample[]) => {
      if (history.length === 0) return;
      const recent = history.slice(-60);
      const fps = recent.map(s => s.totalMs > 0 ? 1000 / s.totalMs : 0);
      fpsHistory = fps;
      const latest = fps[fps.length - 1];
      currentFps = latest > 0 ? latest.toFixed(0) : "—";
    });
  });

  // ─── Chunk flags (BitField) ───────────────────────────────────────────
  // These would come from GPU readback of chunk_flags buffer. For now,
  // derive from what we know on the CPU side.

  const chunkFlags = $derived<BitFieldFlag[]>([
    { label: "RES", value: stats ? stats.residentCount > 0 : undefined, title: "is_resident" },
    { label: "EMP", value: stats ? stats.totalVoxels === 0 : undefined, title: "is_empty" },
    { label: "EMI", value: undefined, title: "has_emissive (needs GPU readback)" },
    { label: "OPQ", value: undefined, title: "is_fully_opaque (needs GPU readback)" },
    { label: "S.M", value: false, title: "stale_mesh (0 — no edits)" },
    { label: "S.S", value: false, title: "stale_summary (0 — no edits)" },
  ]);

  // ─── Derived display values ───────────────────────────────────────────

  const fovDeg = $derived(stats ? stats.cameraFov * 180 / Math.PI : 45);
  const resolution = $derived(stats ? `${stats.viewportWidth} × ${stats.viewportHeight}` : "—");
  const camPos = $derived(stats?.cameraPos
    ? `(${stats.cameraPos[0].toFixed(1)}, ${stats.cameraPos[1].toFixed(1)}, ${stats.cameraPos[2].toFixed(1)})`
    : "—");

  const vertPoolUsedMB = $derived(stats ? (stats.meshVerts * 16) / (1024 * 1024) : 0);
  const idxPoolUsedMB = $derived(stats ? (stats.meshIndices * 4) / (1024 * 1024) : 0);
</script>

<input
  type="file"
  accept=".obj"
  bind:this={fileInputEl}
  onchange={handleFileSelect}
  style="display:none"
/>

<Section sectionId="inspector-model" title="MODEL">
  <Slider
    id="voxel-res"
    label="Resolution"
    min={8}
    max={4096}
    step={2}
    value={voxelResolution}
    decimals={0}
    onValueChange={(v) => { voxelResolution = v; }}
  />
  <ActionButton onclick={triggerFileInput} fullWidth disabled={loadingModel}>
    {loadingModel ? "Voxelizing..." : "Load OBJ"}
  </ActionButton>
  {#if loadError}
    <div class="load-error">{loadError}</div>
  {/if}
</Section>

<Section sectionId="inspector-mode" title="RENDER MODE">
  <SelectField
    options={modeOptions}
    value={currentMode}
    onValueChange={onModeChange}
  />
  <div class="status-row">
    <StatusIndicator
      status={stats?.hasWireframe ? "ok" : "idle"}
      label={stats?.hasWireframe ? "Wire buffers allocated" : "Wire buffers not allocated"}
    />
    <span class="status-label">{stats?.hasWireframe ? "Wire buffers: 128 MB" : "Wire buffers: not allocated"}</span>
  </div>
</Section>

<Section sectionId="inspector-perf" title="PERFORMANCE">
  <CounterRow
    label="FPS"
    value={currentFps}
    history={fpsHistory}
    warn={45}
    danger={30}
  />
</Section>

<Section sectionId="inspector-scene" title="SCENE">
  <PropRow label="Chunks" value={stats?.residentCount?.toLocaleString() ?? "—"} />
  <PropRow label="Voxels" value={stats?.totalVoxels?.toLocaleString() ?? "—"} />
  <BarMeter label="Slot Capacity" value={stats?.residentCount ?? 0} max={1024} />
  <PropRow label="Free Slots" value={stats?.freeSlots?.toLocaleString() ?? "—"} />
  <BitField label="Flags" flags={chunkFlags} />
</Section>

<Section sectionId="inspector-mesh" title="MESH">
  <PropRow label="Quads" value={stats?.meshQuads?.toLocaleString() ?? "—"} />
  <DiffRow label="Vertices" prev={prevVerts} current={stats?.meshVerts ?? 0} />
  <DiffRow label="Indices" prev={prevIndices} current={stats?.meshIndices ?? 0} />
  <BarMeter label="Vertex Pool" value={stats?.meshVerts ?? 0} max={16384} />
  <BarMeter label="Index Pool" value={stats?.meshIndices ?? 0} max={24576} />
</Section>

<Section sectionId="inspector-camera" title="CAMERA">
  <ScrubField
    label="FOV"
    value={fovDeg}
    min={10}
    max={120}
    step={1}
    decimals={1}
    unit="°"
    onValueChange={(v) => RC.setFov(v)}
  />
  <PropRow label="Resolution" value={resolution} />
  <PropRow label="Near / Far" value={stats ? `${stats.cameraNear} / ${stats.cameraFar}` : "—"} />
  <PropRow label="Aspect" value={stats ? stats.cameraAspect.toFixed(2) : "—"} />
  <PropRow label="Position" value={camPos} />
</Section>

<Section sectionId="inspector-debug" title="DEBUG">
  <CheckboxRow
    label="Backface Culling"
    checked={stats?.backfaceCulling ?? true}
    onchange={(v) => RC.setBackfaceCulling(v)}
  />
  <CheckboxRow
    label="Depth Prepass (R-2)"
    checked={stats?.depthPrepassEnabled ?? true}
    onchange={(v) => RC.setDepthPrepass(v)}
  />
  <CheckboxRow
    label="CPU Mesh (bypass GPU R-1)"
    checked={cpuMeshEnabled}
    onchange={(v) => {
      cpuMeshEnabled = v;
      RC.setUseCpuMesh(v);
    }}
  />
  <CheckboxRow
    label="Freeze Cull (debug R-4)"
    checked={freezeCullEnabled}
    onchange={(v) => {
      freezeCullEnabled = v;
      RC.setFreezeCull(v);
    }}
  />
  <CheckboxRow
    label="Frustum Cull"
    checked={frustumCullEnabled}
    onchange={(v) => {
      frustumCullEnabled = v;
      RC.setFrustumCullEnabled(v);
    }}
  />
  <CheckboxRow
    label="Hi-Z Occlusion Cull (R-4)"
    checked={hizCullEnabled}
    onchange={(v) => {
      hizCullEnabled = v;
      RC.setHizCullEnabled(v);
    }}
  />
  <SelectField
    options={giBackendOptions}
    value={giBackendMode}
    onValueChange={onGiBackendChange}
  />
</Section>

<Section sectionId="inspector-gpu" title="GPU MEMORY">
  <BarMeter label="Vertex Pool" value={parseFloat(vertPoolUsedMB.toFixed(1))} max={256} unit="MB" />
  <BarMeter label="Index Pool" value={parseFloat(idxPoolUsedMB.toFixed(1))} max={96} unit="MB" />
  <PropRow label="Wireframe" value={stats?.hasWireframe ? "128 MB allocated" : "not allocated"} />
  <PropRow label="Total Reserved" value="384 MB" />
</Section>

<style>
  .status-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .status-label {
    font-size: 11px;
    color: var(--text-muted);
  }
  .load-error {
    font-size: 11px;
    color: oklch(0.65 0.20 25);
    padding: 4px 0;
  }
</style>
