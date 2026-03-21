<script lang="ts">
  import { Section, PropRow, ActionButton, CounterRow } from "@gestalt/phi";
  import TimelineCanvas from "$lib/components/viz/TimelineCanvas.svelte";
  import PassBreakdownTable from "$lib/components/viz/PassBreakdownTable.svelte";
  import { frameTimeline, diagCounters, diagHistory } from "$lib/stores/timeline";
  import type { FrameSample, DiagCounters } from "$lib/stores/timeline";

  let history = $state<FrameSample[]>([]);
  let paused = $state(false);
  let frozenHistory = $state<FrameSample[]>([]);

  $effect(() => {
    return frameTimeline.subscribe(h => {
      if (!paused) history = h;
    });
  });

  function togglePause() {
    if (!paused) frozenHistory = history;
    paused = !paused;
    if (!paused) frozenHistory = [];
  }

  const displayHistory = $derived(paused ? frozenHistory : history);

  let hoveredFrame = $state<FrameSample | null>(null);

  let diag = $state<DiagCounters | null>(null);
  $effect(() => {
    return diagCounters.subscribe(d => { diag = d; });
  });

  let dh = $state<DiagCounters[]>([]);
  $effect(() => {
    return diagHistory.subscribe(h => { dh = h; });
  });

  const meshletsCulledH    = $derived(dh.map(d => d.meshlets_culled));
  const chunksSkippedH     = $derived(dh.map(d => d.chunks_empty_skipped));
  const versionMismatchesH = $derived(dh.map(d => d.version_mismatches));
  const summaryRebuildsH   = $derived(dh.map(d => d.summary_rebuilds));
  const meshRebuildsH      = $derived(dh.map(d => d.mesh_rebuilds));
  const cascadeRayHitsH    = $derived(dh.map(d => d.cascade_ray_hits));

  const lastMs = $derived(
    displayHistory.length > 0 ? displayHistory[displayHistory.length - 1].totalMs.toFixed(2) : "—"
  );

  const avgMs = $derived(
    displayHistory.length > 0
      ? (displayHistory.reduce((sum, s) => sum + s.totalMs, 0) / displayHistory.length).toFixed(2)
      : "—"
  );

  const peakMs = $derived(
    displayHistory.length > 0
      ? Math.max(...displayHistory.map(s => s.totalMs)).toFixed(2)
      : "—"
  );

  function pct(sorted: number[], p: number): number {
    const idx = Math.ceil((p / 100) * sorted.length) - 1;
    return sorted[Math.max(0, idx)];
  }

  const percentiles = $derived((() => {
    if (displayHistory.length === 0) return null;
    const sorted = displayHistory.map(s => s.totalMs).sort((a, b) => a - b);
    return {
      p50: pct(sorted, 50).toFixed(2),
      p95: pct(sorted, 95).toFixed(2),
      p99: pct(sorted, 99).toFixed(2),
    };
  })());
</script>

<div class="panel-content">
  <div class="section-header" style="margin-bottom: 10px;">Performance</div>

  <Section sectionId="perf-timeline" title="Frame Timeline">
    <TimelineCanvas
      history={displayHistory}
      showLegend={false}
      onHoverFrame={(f) => { hoveredFrame = f; }}
    />
    <PassBreakdownTable history={displayHistory} />
    <ActionButton fullWidth onclick={togglePause}>
      {paused ? "Resume" : "Pause"}
    </ActionButton>
  </Section>

  <Section sectionId="perf-hover" title="Selected Frame">
    {#if hoveredFrame}
      {#each Object.entries(hoveredFrame.passes).sort((a, b) => b[1] - a[1]) as [pass, ms]}
        <PropRow label={pass} value="{ms.toFixed(2)} ms" />
      {/each}
      {#if Object.keys(hoveredFrame.passes).length === 0}
        <PropRow label="Frame total" value="{hoveredFrame.totalMs.toFixed(2)} ms" />
      {/if}
    {:else}
      <span class="hover-hint">Hover over the chart</span>
    {/if}
  </Section>

  <Section sectionId="perf-diag" title="GPU Diagnostics">
    <CounterRow label="Meshlets culled"    value={diag?.meshlets_culled.toLocaleString()      ?? "—"} history={meshletsCulledH} />
    <CounterRow label="Chunks skipped"     value={diag?.chunks_empty_skipped.toLocaleString() ?? "—"} history={chunksSkippedH} />
    <CounterRow label="Version mismatches" value={diag?.version_mismatches.toLocaleString()   ?? "—"} history={versionMismatchesH} danger={1} />
    <CounterRow label="Summary rebuilds"   value={diag?.summary_rebuilds.toLocaleString()     ?? "—"} history={summaryRebuildsH} />
    <CounterRow label="Mesh rebuilds"      value={diag?.mesh_rebuilds.toLocaleString()        ?? "—"} history={meshRebuildsH} />
    <CounterRow label="Cascade ray hits"   value={diag?.cascade_ray_hits.toLocaleString()     ?? "—"} history={cascadeRayHitsH} />
  </Section>

  <Section sectionId="perf-stats" title="Frame Stats">
    <PropRow label="Last frame" value="{lastMs} ms" />
    <PropRow label="Avg frame"  value="{avgMs} ms"  />
    <PropRow label="Peak frame" value="{peakMs} ms" />
    <PropRow label="p50"        value="{percentiles?.p50 ?? '—'} ms" />
    <PropRow label="p95"        value="{percentiles?.p95 ?? '—'} ms" />
    <PropRow label="p99"        value="{percentiles?.p99 ?? '—'} ms" />
    <PropRow label="Samples"    value={String(displayHistory.length)} />
  </Section>
</div>

<style>
  .hover-hint {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }
</style>
