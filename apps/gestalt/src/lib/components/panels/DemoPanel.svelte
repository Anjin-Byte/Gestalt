<script lang="ts">
  import {
    Section, PropRow, ScrubField, SelectField, CheckboxRow,
    ActionButton, StatusIndicator, BarMeter, ToggleGroup,
    CounterRow, Sparkline,
  } from "@gestalt/phi";
  import TimelineCanvas from "$lib/components/viz/TimelineCanvas.svelte";
  import PassBreakdownTable from "$lib/components/viz/PassBreakdownTable.svelte";
  import type { FrameSample } from "$lib/stores/timeline";

  const selectOptions = [
    { value: "option-a", label: "Option A" },
    { value: "option-b", label: "Option B" },
    { value: "option-c", label: "Option C" },
  ];

  // ScrubField demo values
  let basic = $state(0.5);
  let withRange = $state(1.0);
  let integer = $state(64);
  let withUnit = $state(12.5);
  let freeform = $state(3.14159);

  // Controls demo
  let checked = $state(false);
  let checked2 = $state(true);
  let selectedOpt = $state("option-a");
  let lastAction = $state<string | null>(null);

  // BarMeter demo — simulates live-updating GPU pool values
  let slotCount = $state(312);
  let meshMB = $state(79);

  // ToggleGroup demo
  const debugModes = [
    { value: "normal",   label: "Normal"   },
    { value: "bricklet", label: "Bricklet" },
    { value: "emissive", label: "Emissive" },
    { value: "version",  label: "Version"  },
    { value: "meshlet",  label: "Meshlet"  },
  ];
  let debugMode = $state("normal");

  const renderModes = [
    { value: "solid",     label: "Solid"     },
    { value: "wireframe", label: "Wireframe" },
  ];
  let renderMode = $state("solid");

  // TimelineCanvas demo — synthetic GPU frame data
  const DEMO_PASSES = [
    "I-3 Summary Rebuild",
    "R-2 Depth Prepass",
    "R-3 Hi-Z Pyramid",
    "R-4a Chunk Cull",
    "R-4b Meshlet Cull",
    "R-5 Color Pass",
    "R-6 Cascade Build",
    "R-7 Cascade Merge",
  ];

  let demoHistory    = $state<FrameSample[]>([]);
  let demoSimulating = $state(false);
  let demoPaused     = $state(false);
  let demoFrozen     = $state<FrameSample[]>([]);
  let demoSpikeMode  = $state(false);
  let demoR5Cost     = $state(4.5);
  let demoHovered    = $state<FrameSample | null>(null);
  let demoIntervalId: ReturnType<typeof setInterval> | undefined;

  // Sparkline section — self-contained live simulation
  let sparkSimulating = $state(false);
  let sparkIntervalId: ReturnType<typeof setInterval> | undefined;
  let sparkTick = 0;

  interface SparkDemo { stable: number[]; periodic: number[]; spiky: number[]; errors: number[]; }
  let sparkDemo = $state<SparkDemo>({ stable: [], periodic: [], spiky: [], errors: [] });

  const SPARK_CAP = 120;
  function pushSpark(arr: number[], val: number) {
    const next = [...arr, Math.max(0, val)];
    return next.length > SPARK_CAP ? next.slice(1) : next;
  }

  function tickSparkDemo() {
    sparkTick++;
    const t = sparkTick;
    sparkDemo = {
      stable:   pushSpark(sparkDemo.stable,   14200 + (Math.random() - 0.5) * 600),
      periodic: pushSpark(sparkDemo.periodic, 32 + Math.sin(t * 0.12) * 28),
      spiky:    pushSpark(sparkDemo.spiky,    Math.random() < 0.07 ? 40 + Math.random() * 60 : 2 + Math.random() * 6),
      errors:   pushSpark(sparkDemo.errors,   Math.random() < 0.06 ? Math.ceil(Math.random() * 3) : 0),
    };
  }

  function toggleSparkSim() {
    if (sparkSimulating) {
      clearInterval(sparkIntervalId);
      sparkIntervalId = undefined;
      sparkSimulating = false;
      sparkDemo = { stable: [], periodic: [], spiky: [], errors: [] };
      sparkTick = 0;
    } else {
      sparkSimulating = true;
      sparkIntervalId = setInterval(tickSparkDemo, 60);
    }
  }

  function fmtSpark(arr: number[]) {
    const v = arr.at(-1);
    return v != null ? Math.round(v).toLocaleString() : "—";
  }

  interface DemoDiag {
    meshlets_culled: number;
    chunks_skipped: number;
    summary_rebuilds: number;
    mesh_rebuilds: number;
  }
  let demoDiag = $state<DemoDiag | null>(null);
  let demoDiagHistory = $state<DemoDiag[]>([]);

  const meshletsCulledDH    = $derived(demoDiagHistory.map(d => d.meshlets_culled));
  const chunksSkippedDH     = $derived(demoDiagHistory.map(d => d.chunks_skipped));
  const summaryRebuildsDH   = $derived(demoDiagHistory.map(d => d.summary_rebuilds));
  const meshRebuildsDH      = $derived(demoDiagHistory.map(d => d.mesh_rebuilds));

  const demoDisplay = $derived(demoPaused ? demoFrozen : demoHistory);

  function makeDemoSample(): FrameSample {
    const noise = () => (Math.random() - 0.5) * 0.4;
    const bases = [0.3, 0.5, 0.2, 0.4, 0.6, demoR5Cost, 1.8, 0.9];
    const passes: Record<string, number> = {};
    let total = 0;
    for (let i = 0; i < DEMO_PASSES.length; i++) {
      const ms = Math.max(0.05, bases[i] + noise());
      passes[DEMO_PASSES[i]] = ms;
      total += ms;
    }
    if (demoSpikeMode) {
      if (Math.random() < 0.35) total += 6 + Math.random() * 16;
    } else {
      if (Math.random() < 0.04) total += 8 + Math.random() * 12;
    }
    return { totalMs: total, passes };
  }

  function toggleDemoSim() {
    if (demoSimulating) {
      clearInterval(demoIntervalId);
      demoIntervalId = undefined;
      demoSimulating = false;
      demoPaused = false;
      demoFrozen = [];
      demoDiag = null;
      demoDiagHistory = [];
    } else {
      demoSimulating = true;
      demoIntervalId = setInterval(() => {
        const sample = makeDemoSample();
        demoHistory = demoHistory.length >= 240
          ? [...demoHistory.slice(1), sample]
          : [...demoHistory, sample];
        demoDiag = {
          meshlets_culled:  Math.floor(360 + Math.random() * 80),
          chunks_skipped:   Math.floor(16  + Math.random() * 10),
          summary_rebuilds: Math.floor(demoSpikeMode ? 7 + Math.random() * 5 : 1 + Math.random() * 3),
          mesh_rebuilds:    Math.floor(demoSpikeMode ? 5 + Math.random() * 4 : 2 + Math.random() * 3),
        };
        demoDiagHistory = demoDiagHistory.length >= 240
          ? [...demoDiagHistory.slice(1), demoDiag]
          : [...demoDiagHistory, demoDiag];
      }, 16);
    }
  }

  function toggleDemoPause() {
    if (!demoPaused) demoFrozen = demoHistory;
    demoPaused = !demoPaused;
    if (!demoPaused) demoFrozen = [];
  }

  $effect(() => {
    return () => {
      if (demoIntervalId  !== undefined) clearInterval(demoIntervalId);
      if (sparkIntervalId !== undefined) clearInterval(sparkIntervalId);
    };
  });
</script>

<div class="panel-content">
  <div class="section-header" style="margin-bottom: 10px;">Component Demo</div>

  <Section sectionId="demo-scrubfield" title="ScrubField">
    <div class="demo-note">Drag ← → · Click to type · Double-click to reset (if default)</div>

    <div class="demo-group">
      <div class="demo-label">Basic (no bounds)</div>
      <ScrubField
        label="Value"
        value={basic}
        step={0.01}
        decimals={2}
        onValueChange={(v) => (basic = v)}
      />
    </div>

    <div class="demo-group">
      <div class="demo-label">With range + default</div>
      <ScrubField
        label="Exposure"
        value={withRange}
        defaultValue={1.0}
        min={0.6}
        max={2.5}
        step={0.05}
        decimals={2}
        onValueChange={(v) => (withRange = v)}
      />
    </div>

    <div class="demo-group">
      <div class="demo-label">Integer step</div>
      <ScrubField
        label="Grid Dim"
        value={integer}
        defaultValue={64}
        min={8}
        max={256}
        step={8}
        decimals={0}
        onValueChange={(v) => (integer = v)}
      />
    </div>

    <div class="demo-group">
      <div class="demo-label">With unit</div>
      <ScrubField
        label="Distance"
        value={withUnit}
        defaultValue={10.0}
        min={0.1}
        max={100.0}
        step={0.1}
        decimals={1}
        unit="m"
        onValueChange={(v) => (withUnit = v)}
      />
    </div>

    <div class="demo-group">
      <div class="demo-label">Freeform (no bounds)</div>
      <ScrubField
        label="Pi-ish"
        value={freeform}
        step={0.00001}
        decimals={5}
        onValueChange={(v) => (freeform = v)}
      />
    </div>
  </Section>

  <Section sectionId="demo-proprow" title="PropRow">
    <div class="demo-note">Hover to reveal copy · Click to copy to clipboard</div>
    <PropRow label="Renderer" value="WebGPU" />
    <PropRow label="Chunk size" value="64³ voxels" />
    <PropRow label="Build hash" value="a3f9c2d1e8b047" />
    <PropRow label="Max invocations" value="256" />
    <PropRow label="Max storage" value="128 MB" />
  </Section>

  <Section sectionId="demo-checkboxrow" title="CheckboxRow">
    <div class="demo-note">Full-row click target · Blue fill when checked · Accessible focus ring</div>

    <CheckboxRow
      label="Unchecked (default)"
      {checked}
      onchange={(v) => (checked = v)}
    />
    <CheckboxRow
      label="Pre-checked"
      checked={checked2}
      onchange={(v) => (checked2 = v)}
    />
    <CheckboxRow
      label="Disabled"
      disabled
      onchange={() => {}}
    />
    <CheckboxRow
      label="Disabled + checked"
      checked={true}
      disabled
      onchange={() => {}}
    />

    <PropRow label="unchecked" value={String(checked)} />
    <PropRow label="pre-checked" value={String(checked2)} />
  </Section>

  <Section sectionId="demo-actionbutton" title="ActionButton">
    <div class="demo-note">Raised surface · Hover lightens · Disabled at 40% opacity</div>

    <div class="demo-group">
      <div class="demo-label">Inline (auto-width)</div>
      <div class="btn-row">
        <ActionButton onclick={() => (lastAction = "primary")}>Primary</ActionButton>
        <ActionButton onclick={() => (lastAction = "secondary")}>Secondary</ActionButton>
        <ActionButton disabled onclick={() => {}}>Disabled</ActionButton>
      </div>
    </div>

    <div class="demo-group">
      <div class="demo-label">Full-width</div>
      <ActionButton fullWidth onclick={() => (lastAction = "full-width")}>
        Full Width Action
      </ActionButton>
    </div>

    {#if lastAction}
      <PropRow label="last fired" value={lastAction} />
    {/if}
  </Section>

  <Section sectionId="demo-select" title="SelectField">
    <div class="demo-note">Dropdown via Bits UI · Matches trigger width · Animated open</div>

    <div class="field-row">
      <span class="label">Inline variant</span>
      <div style="width: 120px;">
        <SelectField
          options={selectOptions}
          value={selectedOpt}
          inline
          onValueChange={(v) => (selectedOpt = v)}
        />
      </div>
    </div>

    <div class="demo-group" style="margin-top: 10px;">
      <div class="demo-label">Full-width</div>
      <SelectField
        options={selectOptions}
        value={selectedOpt}
        onValueChange={(v) => (selectedOpt = v)}
      />
    </div>

    <PropRow label="selected" value={selectedOpt} />
  </Section>

  <Section sectionId="demo-statusindicator" title="StatusIndicator">
    <div class="demo-note">Live pulse on ok · Static dot for warning/error · Idle is dimmed</div>

    <div class="si-grid">
      <StatusIndicator status="ok"      label="Connected"    />
      <StatusIndicator status="warning" label="Degraded"     />
      <StatusIndicator status="error"   label="Device lost"  />
      <StatusIndicator status="idle"    label="Idle"         />
    </div>

    <div class="demo-group" style="margin-top: 10px;">
      <div class="demo-label">Dot only (no label)</div>
      <div class="si-row">
        <StatusIndicator status="ok"      />
        <StatusIndicator status="warning" />
        <StatusIndicator status="error"   />
        <StatusIndicator status="idle"    />
      </div>
    </div>
  </Section>

  <Section sectionId="demo-barmeter" title="BarMeter">
    <div class="demo-note">Blue → amber (≥80%) → red (≥90%) · Threshold tick · Animated fill</div>

    <BarMeter label="Chunk slots" value={slotCount} max={1024} />
    <BarMeter label="Mesh pool"   value={meshMB}    max={256}   unit="MB" />
    <BarMeter label="Near full"   value={840}  max={1024} />
    <BarMeter label="Critical"    value={940}  max={1024} />

    <div class="demo-group" style="margin-top: 10px;">
      <div class="demo-label">Scrub to simulate live data</div>
      <ScrubField
        label="Slot count"
        value={slotCount}
        min={0}
        max={1024}
        step={1}
        decimals={0}
        onValueChange={(v) => (slotCount = v)}
      />
      <ScrubField
        label="Mesh MB"
        value={meshMB}
        min={0}
        max={256}
        step={1}
        decimals={0}
        onValueChange={(v) => (meshMB = v)}
      />
    </div>
  </Section>

  <Section sectionId="demo-togglegroup" title="ToggleGroup">
    <div class="demo-note">Equal-width · Arrow key navigation · Inset focus ring</div>

    <div class="demo-group">
      <div class="demo-label">2 options</div>
      <ToggleGroup
        options={renderModes}
        value={renderMode}
        label="Render mode"
        onValueChange={(v) => (renderMode = v)}
      />
    </div>

    <div class="demo-group">
      <div class="demo-label">5 options (debug render modes)</div>
      <ToggleGroup
        options={debugModes}
        value={debugMode}
        label="Debug mode"
        onValueChange={(v) => (debugMode = v)}
      />
    </div>

    <PropRow label="render mode" value={renderMode} />
    <PropRow label="debug mode"  value={debugMode}  />
  </Section>

  <Section sectionId="demo-sparkline" title="Sparkline / CounterRow">
    <div class="demo-note">
      Hit Start · Four archetypes — each reveals a pattern a single number cannot
    </div>

    <div class="spark-blocks">
      <div class="spark-block">
        <div class="spark-block-header">
          <span class="spark-block-name">Stable</span>
          <span class="spark-block-desc">low-variance noise around a mean</span>
          <span class="spark-block-val">{fmtSpark(sparkDemo.stable)}</span>
        </div>
        <Sparkline values={sparkDemo.stable} height={56} />
      </div>

      <div class="spark-block">
        <div class="spark-block-header">
          <span class="spark-block-name">Periodic</span>
          <span class="spark-block-desc">slow oscillation — cascade update cycle</span>
          <span class="spark-block-val">{fmtSpark(sparkDemo.periodic)}</span>
        </div>
        <Sparkline values={sparkDemo.periodic} height={56} />
      </div>

      <div class="spark-block">
        <div class="spark-block-header">
          <span class="spark-block-name">Spiky</span>
          <span class="spark-block-desc">rare high bursts, baseline near zero</span>
          <span class="spark-block-val">{fmtSpark(sparkDemo.spiky)}</span>
        </div>
        <Sparkline values={sparkDemo.spiky} height={56} />
      </div>

      <div class="spark-block">
        <div class="spark-block-header">
          <span class="spark-block-name">Error counter</span>
          <span class="spark-block-desc">should be zero · dot turns red on any hit</span>
          <span
            class="spark-block-val"
            class:spark-val-danger={(sparkDemo.errors.at(-1) ?? 0) >= 1}
          >{fmtSpark(sparkDemo.errors)}</span>
        </div>
        <Sparkline values={sparkDemo.errors} height={56} danger={1} />
      </div>
    </div>

    <ActionButton onclick={toggleSparkSim}>
      {sparkSimulating ? "Stop" : "Start"}
    </ActionButton>

    <div class="demo-group" style="margin-top: 14px;">
      <div class="demo-label">Compact form — CounterRow as used in Performance Panel</div>
      <CounterRow label="Meshlets culled"    value={fmtSpark(sparkDemo.stable)}   history={sparkDemo.stable} />
      <CounterRow label="Chunks skipped"     value={fmtSpark(sparkDemo.periodic)} history={sparkDemo.periodic} />
      <CounterRow label="Mesh rebuilds"      value={fmtSpark(sparkDemo.spiky)}    history={sparkDemo.spiky} />
      <CounterRow label="Version mismatches" value={fmtSpark(sparkDemo.errors)}   history={sparkDemo.errors} danger={1} />
    </div>
  </Section>

  <Section sectionId="demo-timelinecanvas" title="TimelineCanvas">
    <div class="demo-note">
      Hover frames to inspect passes · Red tint = over 16ms budget ·
      Purple line = R-5 target · PassBreakdownTable replaces canvas legend ·
      Pause freezes chart + stats · Spike mode forces over-budget frames
    </div>

    <!-- Chart — showLegend=false, PassBreakdownTable takes over -->
    <TimelineCanvas
      history={demoDisplay}
      showLegend={false}
      budgetLines={[{ ms: demoR5Cost, label: "R-5 target", color: "oklch(0.73 0.16 290 / 55%)" }]}
      onHoverFrame={(f) => { demoHovered = f; }}
    />
    <PassBreakdownTable history={demoDisplay} />

    <!-- Selected frame — driven by onHoverFrame callback -->
    <Section sectionId="demo-tc-hover" title="Selected Frame">
      {#if demoHovered}
        {#each Object.entries(demoHovered.passes).sort((a, b) => b[1] - a[1]) as [pass, ms]}
          <PropRow label={pass} value="{ms.toFixed(2)} ms" />
        {/each}
      {:else}
        <span class="demo-hover-hint">Hover over the chart</span>
      {/if}
    </Section>

    <!-- Simulation controls -->
    <div class="demo-group">
      <div class="demo-label">R-5 Color Pass cost — moves the purple budget line + shifts PassBreakdownTable color</div>
      <ScrubField
        label="ms"
        value={demoR5Cost}
        min={0.5}
        max={20.0}
        step={0.1}
        decimals={1}
        onValueChange={(v) => (demoR5Cost = v)}
      />
    </div>

    <CheckboxRow
      label="Spike mode — frequent over-budget frames"
      checked={demoSpikeMode}
      onchange={(v) => (demoSpikeMode = v)}
    />

    <div class="btn-row">
      <ActionButton onclick={toggleDemoSim}>
        {demoSimulating ? "Stop" : "Start"}
      </ActionButton>
      <ActionButton onclick={toggleDemoPause} disabled={!demoSimulating}>
        {demoPaused ? "Resume" : "Pause"}
      </ActionButton>
    </div>

    <!-- Synthetic diagnostic counters — mirrors real DiagCounters layout -->
    {#if demoDiag}
      <Section sectionId="demo-tc-diag" title="Synthetic Counters">
        <div class="demo-note">Mirrors the GPU Diagnostics section in PerformancePanel. Rebuilds spike when Spike mode is on.</div>
        <CounterRow label="Meshlets culled"  value={demoDiag.meshlets_culled.toLocaleString()}  history={meshletsCulledDH} />
        <CounterRow label="Chunks skipped"   value={demoDiag.chunks_skipped.toLocaleString()}   history={chunksSkippedDH} />
        <CounterRow label="Summary rebuilds" value={demoDiag.summary_rebuilds.toLocaleString()} history={summaryRebuildsDH} />
        <CounterRow label="Mesh rebuilds"    value={demoDiag.mesh_rebuilds.toLocaleString()}    history={meshRebuildsDH} />
      </Section>
    {/if}
  </Section>

  <Section sectionId="demo-sections" title="Section (meta)">
    <div class="demo-note">
      Sections are collapsible. State persists to localStorage per <code>sectionId</code>.
    </div>
    <Section sectionId="demo-nested" title="Nested Section">
      <PropRow label="Nesting depth" value="2" />
      <PropRow label="Works" value="yes" />
    </Section>
  </Section>
</div>

<style>
  .demo-note {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    margin-bottom: 8px;
    line-height: 1.6;
  }

  .demo-group {
    margin-bottom: 8px;
  }

  .demo-group:last-child {
    margin-bottom: 0;
  }

  .demo-label {
    font-size: 10px;
    color: var(--text-faint);
    margin-bottom: 2px;
    font-family: var(--font-mono);
  }

  .field-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .btn-row {
    display: flex;
    gap: 8px;
  }

  /* StatusIndicator demo layouts */
  .si-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .si-row {
    display: flex;
    gap: 12px;
    align-items: center;
  }

  code {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--color-meta);
    background: var(--fill-lo);
    padding: 1px 4px;
    border-radius: 2px;
  }

  .demo-hover-hint {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }

  .spark-blocks {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 10px;
  }

  .spark-block {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .spark-block-header {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }

  .spark-block-name {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-mid);
    flex-shrink: 0;
  }

  .spark-block-desc {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .spark-block-val {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-mid);
    flex-shrink: 0;
  }

  .spark-val-danger {
    color: var(--color-destructive);
  }
</style>
