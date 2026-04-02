<script lang="ts">
  import {
    Section, PropRow, ScrubField, SelectField, CheckboxRow,
    ActionButton, StatusIndicator, BarMeter, ToggleGroup,
    CounterRow, Sparkline,
    TreeList,
    DiffRow, BitField,
  } from "@gestalt/phi";
  import type { BitFieldFlag } from "@gestalt/phi";
  import type {
    TreeListDomain, TreeListItem, TreeListColumnDef, ContextMenuItem,
  } from "@gestalt/phi";
  import { Eye, EyeOff, Camera, CameraOff, Check, X, Circle } from "lucide-svelte";
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

  // ─── TreeList Demo ──────────────────────────────────────────────────────

  // Demo 1: Scene graph with toggles, faded rows, drag, and badges
  interface SceneNode {
    id: string;
    name: string;
    group: string;
    visible: boolean;
    renderable: boolean;
    status: "ok" | "warning" | "error" | "idle";
    faded?: boolean;
    badge?: "linked" | "override" | "asset";
    depth?: number;
  }

  let sceneNodes = $state<SceneNode[]>([
    { id: "cam-main",   name: "Main Camera",       group: "Cameras",    visible: true,  renderable: true,  status: "ok" },
    { id: "cam-debug",  name: "Debug Camera",       group: "Cameras",    visible: false, renderable: false, status: "idle", faded: true },
    { id: "sun",        name: "Sun Light",          group: "Lights",     visible: true,  renderable: true,  status: "ok" },
    { id: "point-a",    name: "Point Fill A",       group: "Lights",     visible: true,  renderable: true,  status: "ok", depth: 1 },
    { id: "point-b",    name: "Point Fill B",       group: "Lights",     visible: false, renderable: true,  status: "warning", faded: true, depth: 1 },
    { id: "sponza",     name: "Sponza",             group: "Geometry",   visible: true,  renderable: true,  status: "ok", badge: "asset" },
    { id: "chunk-0",    name: "Chunk [0,0,0]",      group: "Geometry",   visible: true,  renderable: true,  status: "ok", depth: 1 },
    { id: "chunk-1",    name: "Chunk [1,0,0]",      group: "Geometry",   visible: true,  renderable: true,  status: "ok", depth: 1 },
    { id: "chunk-2",    name: "Chunk [0,1,0]",      group: "Geometry",   visible: true,  renderable: true,  status: "warning", depth: 1 },
    { id: "chunk-3",    name: "Chunk [1,1,0]",      group: "Geometry",   visible: false, renderable: false, status: "idle", faded: true, depth: 1 },
    { id: "chunk-4",    name: "Chunk [0,0,1]",      group: "Geometry",   visible: true,  renderable: true,  status: "ok", depth: 1 },
    { id: "grid-helper",name: "Grid Helper",        group: "Debug",      visible: true,  renderable: false, status: "idle", badge: "linked" },
    { id: "axes-helper",name: "Axes Helper",        group: "Debug",      visible: true,  renderable: false, status: "idle" },
    { id: "bbox-vis",   name: "Chunk AABB Viz",     group: "Debug",      visible: false, renderable: false, status: "idle", faded: true },
    { id: "cascade-vis",name: "Cascade Probe Viz",  group: "Debug",      visible: false, renderable: false, status: "idle", faded: true, badge: "override" },
  ]);

  let sceneSelectedId = $state<string | null>(null);
  let sceneActiveId = $state<string | null>(null);
  let sceneLastAction = $state("");

  const sceneColumns: TreeListColumnDef[] = [
    { id: "visible",    width: 22, label: "Visible in viewport" },
    { id: "renderable", width: 22, label: "Include in render" },
    { id: "status",     width: 22, label: "Status" },
  ];

  const sceneDomain: TreeListDomain<SceneNode[]> = {
    domainId: "demo-scene",
    columns: sceneColumns,
    rows(data: SceneNode[]): TreeListItem[] {
      const groups = [...new Set(data.map(n => n.group))];
      const items: TreeListItem[] = [];
      for (const g of groups) {
        const members = data.filter(n => n.group === g);
        items.push({ kind: "group", id: g, label: g, count: members.length });
        for (const n of members) {
          items.push({
            kind: "row",
            id: n.id,
            groupId: g,
            label: n.name,
            depth: n.depth,
            faded: n.faded,
            statusBadge: n.badge,
            draggable: true,
            renameable: true,
            cells: [
              { type: "toggle", value: n.visible,    icon: n.visible ? Eye : EyeOff, propagatable: true },
              { type: "toggle", value: n.renderable, icon: n.renderable ? Camera : CameraOff },
              { type: "status", status: n.status },
            ],
          });
        }
      }
      return items;
    },
    onSelect(selectedId: string) {
      sceneLastAction = `selected: ${selectedId}`;
    },
    onToggle(rowId: string, columnId: string, value: boolean, propagate: boolean) {
      sceneLastAction = `toggle ${columnId}=${value} on ${rowId}${propagate ? " (propagate)" : ""}`;
      sceneNodes = sceneNodes.map(n => {
        if (n.id !== rowId) return n;
        const updated = { ...n, [columnId]: value };
        // When hiding, also fade
        if (columnId === "visible") {
          updated.faded = !value;
        }
        return updated;
      });
    },
    onDrop(dragId: string, targetId: string, zone) {
      sceneLastAction = `drop ${dragId} → ${zone} ${targetId}`;
    },
    onRename(id: string, newLabel: string) {
      sceneLastAction = `rename ${id} → "${newLabel}"`;
      sceneNodes = sceneNodes.map(n =>
        n.id === id ? { ...n, name: newLabel } : n
      );
    },
    onDelete(ids: string[]) {
      sceneLastAction = `delete [${ids.join(", ")}]`;
      sceneNodes = sceneNodes.filter(n => !ids.includes(n.id));
    },
    onDuplicate(ids: string[]) {
      sceneLastAction = `duplicate [${ids.join(", ")}]`;
      const dupes: SceneNode[] = [];
      for (const id of ids) {
        const src = sceneNodes.find(n => n.id === id);
        if (!src) continue;
        const newId = `${src.id}-copy-${Date.now()}`;
        dupes.push({ ...src, id: newId, name: `${src.name} Copy` });
      }
      sceneNodes = [...sceneNodes, ...dupes];
    },
    getContextItems(id: string): ContextMenuItem[] {
      const node = sceneNodes.find(n => n.id === id);
      if (!node) return [];
      return [
        { id: "rename", label: "Rename", shortcut: "F2" },
        { id: "duplicate", label: "Duplicate", shortcut: "⌘D" },
        { id: "toggle-visible", label: node.visible ? "Hide" : "Show", shortcut: "H" },
        { id: "focus", label: "Focus in Viewport" },
        { id: "delete", label: "Delete", shortcut: "Del", danger: true, separator: true },
      ];
    },
    onContextAction(rowId: string, actionId: string) {
      sceneLastAction = `context: ${actionId} on ${rowId}`;
      switch (actionId) {
        case "rename":
          // The TreeList's F2 path handles this — but from context menu
          // we just log it. A real impl would trigger inline rename.
          break;
        case "duplicate":
          sceneDomain.onDuplicate?.([rowId]);
          return; // onDuplicate already sets sceneLastAction
        case "delete":
          sceneDomain.onDelete?.([rowId]);
          return;
        case "toggle-visible": {
          const node = sceneNodes.find(n => n.id === rowId);
          if (node) {
            sceneDomain.onToggle?.(rowId, "visible", !node.visible, false);
          }
          return;
        }
        case "focus":
          sceneLastAction = `focus viewport on ${rowId}`;
          break;
      }
    },
  };

  // Demo 2: GPU buffer pool with live-updating sparklines and mono cells
  interface PoolSlot {
    id: string;
    name: string;
    sizeMB: number;
    version: number;
    status: "ok" | "warning" | "error" | "idle";
    ageHistory: number[];
  }

  let poolSimulating = $state(false);
  let poolIntervalId: ReturnType<typeof setInterval> | undefined;

  let poolSlots = $state<PoolSlot[]>([
    { id: "buf-0", name: "Chunk Vertices",   sizeMB: 32,  version: 14, status: "ok",      ageHistory: [] },
    { id: "buf-1", name: "Chunk Indices",    sizeMB: 16,  version: 14, status: "ok",      ageHistory: [] },
    { id: "buf-2", name: "Indirect Args",    sizeMB: 0.5, version: 7,  status: "ok",      ageHistory: [] },
    { id: "buf-3", name: "Cascade Probes",   sizeMB: 8,   version: 3,  status: "warning", ageHistory: [] },
    { id: "buf-4", name: "Hi-Z Pyramid",     sizeMB: 4,   version: 1,  status: "ok",      ageHistory: [] },
    { id: "buf-5", name: "Visibility Flags",  sizeMB: 0.25,version: 14, status: "ok",     ageHistory: [] },
    { id: "buf-6", name: "Stale Upload",      sizeMB: 64,  version: 0,  status: "error",  ageHistory: [] },
    { id: "buf-7", name: "Free Slot",         sizeMB: 0,   version: 0,  status: "idle",   ageHistory: [] },
  ]);

  let poolSelectedId = $state<string | null>(null);
  let poolActiveId = $state<string | null>(null);

  function tickPool() {
    poolSlots = poolSlots.map(s => {
      const age = s.ageHistory.length > 0 ? (s.ageHistory.at(-1) ?? 0) : 0;
      const next = s.status === "idle" ? 0
        : s.status === "error" ? age + 2 + Math.random() * 3
        : Math.max(0, age + (Math.random() - 0.45) * 2);
      const history = [...s.ageHistory, next].slice(-80);
      return {
        ...s,
        version: s.status !== "idle" ? s.version + (Math.random() < 0.15 ? 1 : 0) : s.version,
        ageHistory: history,
        status: next > 20 ? "error" as const
          : next > 10 ? "warning" as const
          : s.status === "idle" ? "idle" as const
          : "ok" as const,
      };
    });
  }

  function togglePoolSim() {
    if (poolSimulating) {
      clearInterval(poolIntervalId);
      poolIntervalId = undefined;
      poolSimulating = false;
    } else {
      poolSimulating = true;
      poolIntervalId = setInterval(tickPool, 100);
    }
  }

  const poolColumns: TreeListColumnDef[] = [
    { id: "status",  width: 22, label: "Buffer status" },
    { id: "version", width: 36, label: "Version", hideBelow: 280 },
    { id: "size",    width: 44, label: "Size (MB)", hideBelow: 320 },
    { id: "age",     width: 52, label: "Age trend", hideBelow: 360 },
  ];

  const poolDomain: TreeListDomain<PoolSlot[]> = {
    domainId: "demo-pool",
    columns: poolColumns,
    rows(data: PoolSlot[]): TreeListItem[] {
      const active = data.filter(s => s.status !== "idle");
      const idle = data.filter(s => s.status === "idle");
      const totalMB = active.reduce((sum, s) => sum + s.sizeMB, 0);
      const items: TreeListItem[] = [];

      items.push({
        kind: "group",
        id: "active-bufs",
        label: "Active Buffers",
        count: active.length,
        aggregate: { value: totalMB, max: 256, unit: "MB" },
      });
      for (const s of active) {
        items.push({
          kind: "row",
          id: s.id,
          groupId: "active-bufs",
          label: s.name,
          faded: s.status === "error",
          cells: [
            { type: "status", status: s.status },
            { type: "mono", value: `v${s.version}` },
            { type: "mono", value: s.sizeMB >= 1 ? `${s.sizeMB}` : `${(s.sizeMB * 1024).toFixed(0)} KB` },
            { type: "spark", values: s.ageHistory, warn: 10, danger: 20 },
          ],
        });
      }

      if (idle.length > 0) {
        items.push({ kind: "group", id: "idle-bufs", label: "Free Slots", count: idle.length });
        for (const s of idle) {
          items.push({
            kind: "row",
            id: s.id,
            groupId: "idle-bufs",
            label: s.name,
            faded: true,
            cells: [
              { type: "status", status: "idle" },
              { type: "mono", value: "—" },
              { type: "mono", value: "—" },
              { type: "spark", values: [] },
            ],
          });
        }
      }

      return items;
    },
    onSelect(_id: string) {},
    getContextItems(id: string): ContextMenuItem[] {
      const slot = poolSlots.find(s => s.id === id);
      if (!slot) return [];
      const isActive = slot.status !== "idle";
      return [
        { id: "inspect", label: "Inspect Buffer" },
        { id: "copy-addr", label: "Copy GPU Address", shortcut: "⌘C" },
        { id: "force-rebuild", label: "Force Rebuild", disabled: slot.status === "idle" },
        { id: "evict", label: "Evict to Free List", disabled: !isActive, separator: true },
        { id: "resize", label: "Resize Allocation…", disabled: slot.status === "idle" },
        { id: "destroy", label: "Destroy Buffer", danger: true, separator: true, disabled: slot.status === "idle" },
      ];
    },
    onContextAction(rowId: string, actionId: string) {
      const slot = poolSlots.find(s => s.id === rowId);
      switch (actionId) {
        case "evict":
          if (slot) {
            poolSlots = poolSlots.map(s =>
              s.id === rowId ? { ...s, status: "idle" as const, sizeMB: 0, version: 0, ageHistory: [] } : s
            );
          }
          break;
        case "force-rebuild":
          if (slot) {
            poolSlots = poolSlots.map(s =>
              s.id === rowId ? { ...s, version: s.version + 1, status: "ok" as const, ageHistory: [] } : s
            );
          }
          break;
        case "destroy":
          poolSlots = poolSlots.filter(s => s.id !== rowId);
          break;
      }
    },
  };

  // Demo 3: Render pass pipeline — exercises deep nesting and many columns
  interface RenderPass {
    id: string;
    name: string;
    phase: string;
    enabled: boolean;
    ms: number;
    reads: string;
    writes: string;
  }

  const renderPasses: RenderPass[] = [
    { id: "dp",    name: "Depth Prepass",     phase: "Geometry",   enabled: true,  ms: 0.52,  reads: "—",           writes: "depth" },
    { id: "hiz",   name: "Hi-Z Build",        phase: "Geometry",   enabled: true,  ms: 0.18,  reads: "depth",       writes: "hi-z pyramid" },
    { id: "cull",  name: "Occlusion Cull",    phase: "Geometry",   enabled: true,  ms: 0.31,  reads: "hi-z",        writes: "indirect args" },
    { id: "color", name: "Color Pass",        phase: "Shading",    enabled: true,  ms: 4.20,  reads: "indirect",    writes: "color RT" },
    { id: "cb",    name: "Cascade Build",     phase: "Lighting",   enabled: true,  ms: 1.85,  reads: "depth, color",writes: "probe data" },
    { id: "cm",    name: "Cascade Merge",     phase: "Lighting",   enabled: true,  ms: 0.92,  reads: "probe data",  writes: "GI buffer" },
    { id: "comp",  name: "Composite",         phase: "Post",       enabled: true,  ms: 0.14,  reads: "color, GI",   writes: "final RT" },
    { id: "taa",   name: "TAA",               phase: "Post",       enabled: false, ms: 0.00,  reads: "final RT",    writes: "resolved RT" },
    { id: "bloom", name: "Bloom",             phase: "Post",       enabled: false, ms: 0.00,  reads: "resolved RT", writes: "resolved RT" },
    { id: "dbg",   name: "Debug Overlay",     phase: "Debug",      enabled: true,  ms: 0.08,  reads: "final RT",    writes: "swapchain" },
  ];

  let passSelectedId = $state<string | null>(null);
  let passActiveId = $state<string | null>(null);
  let passNodes = $state(renderPasses);

  const passColumns: TreeListColumnDef[] = [
    { id: "enabled", width: 22, label: "Pass enabled" },
    { id: "ms",      width: 48, label: "GPU time (ms)" },
    { id: "reads",   width: 64, label: "Reads", hideBelow: 340 },
    { id: "writes",  width: 64, label: "Writes", hideBelow: 400 },
  ];

  const passDomain: TreeListDomain<RenderPass[]> = {
    domainId: "demo-passes",
    columns: passColumns,
    rows(data: RenderPass[]): TreeListItem[] {
      const phases = [...new Set(data.map(p => p.phase))];
      const items: TreeListItem[] = [];
      for (const phase of phases) {
        const members = data.filter(p => p.phase === phase);
        const totalMs = members.filter(p => p.enabled).reduce((s, p) => s + p.ms, 0);
        items.push({
          kind: "group",
          id: phase,
          label: `${phase} (${totalMs.toFixed(1)} ms)`,
          count: members.length,
        });
        for (const p of members) {
          items.push({
            kind: "row",
            id: p.id,
            groupId: phase,
            label: p.name,
            faded: !p.enabled,
            cells: [
              { type: "toggle", value: p.enabled, icon: p.enabled ? Check : X },
              { type: "mono", value: p.enabled ? `${p.ms.toFixed(2)}` : "—" },
              { type: "mono", value: p.reads },
              { type: "mono", value: p.writes },
            ],
          });
        }
      }
      return items;
    },
    onToggle(rowId, _columnId, value) {
      passNodes = passNodes.map(p =>
        p.id === rowId ? { ...p, enabled: value } : p
      );
    },
    getContextItems(id: string): ContextMenuItem[] {
      const pass = passNodes.find(p => p.id === id);
      if (!pass) return [];
      return [
        { id: "toggle", label: pass.enabled ? "Disable Pass" : "Enable Pass" },
        { id: "solo", label: "Solo (disable all others)" },
        { id: "profile", label: "Profile This Pass", shortcut: "P" },
        { id: "inspect-reads", label: `Inspect Reads: ${pass.reads}`, disabled: pass.reads === "—" },
        { id: "inspect-writes", label: `Inspect Writes: ${pass.writes}`, separator: true },
        { id: "move-up", label: "Move Up", shortcut: "⌥↑", disabled: passNodes.indexOf(pass) === 0 },
        { id: "move-down", label: "Move Down", shortcut: "⌥↓", disabled: passNodes.indexOf(pass) === passNodes.length - 1 },
      ];
    },
    onContextAction(rowId: string, actionId: string) {
      switch (actionId) {
        case "toggle": {
          const pass = passNodes.find(p => p.id === rowId);
          if (pass) {
            passNodes = passNodes.map(p =>
              p.id === rowId ? { ...p, enabled: !p.enabled } : p
            );
          }
          break;
        }
        case "solo":
          passNodes = passNodes.map(p => ({
            ...p,
            enabled: p.id === rowId,
          }));
          break;
        case "move-up": {
          const idx = passNodes.findIndex(p => p.id === rowId);
          if (idx > 0) {
            const copy = [...passNodes];
            [copy[idx - 1], copy[idx]] = [copy[idx], copy[idx - 1]];
            passNodes = copy;
          }
          break;
        }
        case "move-down": {
          const idx = passNodes.findIndex(p => p.id === rowId);
          if (idx < passNodes.length - 1) {
            const copy = [...passNodes];
            [copy[idx], copy[idx + 1]] = [copy[idx + 1], copy[idx]];
            passNodes = copy;
          }
          break;
        }
      }
    },
  };

  // TreeList selection change handlers
  function onSceneSelChange(sel: string | null, act: string | null) {
    sceneSelectedId = sel;
    sceneActiveId = act;
  }
  function onPoolSelChange(sel: string | null, act: string | null) {
    poolSelectedId = sel;
    poolActiveId = act;
  }
  function onPassSelChange(sel: string | null, act: string | null) {
    passSelectedId = sel;
    passActiveId = act;
  }

  $effect(() => {
    return () => {
      if (demoIntervalId  !== undefined) clearInterval(demoIntervalId);
      if (sparkIntervalId !== undefined) clearInterval(sparkIntervalId);
      if (poolIntervalId  !== undefined) clearInterval(poolIntervalId);
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

  <Section sectionId="demo-diffrow" title="DiffRow">
    <div class="demo-note">
      Prev → Current with delta indicator · Green for improvement · Yellow for regression ·
      invertWarning flips the logic (e.g. FPS where lower = bad)
    </div>
    <DiffRow label="Frame time" prev={8.2} current={12.5} unit="ms" decimals={1} />
    <DiffRow label="Meshlets culled" prev={340} current={280} />
    <DiffRow label="Mesh rebuilds" prev={3} current={3} />
    <DiffRow label="FPS" prev={60} current={45} invertWarning />
    <DiffRow label="Buffer usage" prev={128} current={96} unit="MB" />
    <DiffRow label="Cascade rays" prev={14200} current={18400} />
  </Section>

  <Section sectionId="demo-bitfield" title="BitField">
    <div class="demo-note">
      Pipeline state flags · Green = on · Dim = off · Yellow = unknown/tri-state ·
      Hover for full label via title attribute
    </div>
    <BitField label="Render State" flags={[
      { label: "ZW", value: true, title: "Depth Write" },
      { label: "ZT", value: true, title: "Depth Test" },
      { label: "BFC", value: true, title: "Backface Cull" },
      { label: "ST", value: false, title: "Stencil Test" },
      { label: "BL", value: false, title: "Blending" },
      { label: "SC", value: true, title: "Scissor" },
    ]} />
    <BitField label="Chunk Flags" flags={[
      { label: "V", value: true, title: "Visible" },
      { label: "D", value: false, title: "Dirty" },
      { label: "L", value: true, title: "Loaded" },
      { label: "M", value: undefined, title: "Meshed (pending)" },
      { label: "C", value: true, title: "Culled" },
      { label: "U", value: false, title: "Uploaded" },
      { label: "E", value: true, title: "Eviction Candidate" },
      { label: "R", value: undefined, title: "Readback Pending" },
    ]} />
    <BitField label="Pass Enable" flags={[
      { label: "DP", value: true, title: "Depth Prepass" },
      { label: "HZ", value: true, title: "Hi-Z Build" },
      { label: "OC", value: true, title: "Occlusion Cull" },
      { label: "CO", value: true, title: "Color Pass" },
      { label: "CB", value: true, title: "Cascade Build" },
      { label: "CM", value: true, title: "Cascade Merge" },
      { label: "TA", value: false, title: "TAA" },
      { label: "BM", value: false, title: "Bloom" },
    ]} />
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

  <!-- ── TreeList Demos ─────────────────────────────────────────────────── -->

  <Section sectionId="demo-treelist-scene" title="TreeList — Scene Graph" card>
    <div class="demo-note">
      Right-click for context menu · Toggle columns (👁 / 📷) ·
      Shift+click propagates · Drag rows · Filter by name ·
      ↑↓ navigate · Double-click / F2 rename · Del remove · ⌘D duplicate
    </div>

    <div class="treelist-demo-frame">
      <TreeList
        domain={sceneDomain}
        data={sceneNodes}
        selectedId={sceneSelectedId}
        activeId={sceneActiveId}
        onselectionchange={onSceneSelChange}
      />
    </div>

    {#if sceneLastAction}
      <PropRow label="last action" value={sceneLastAction} />
    {/if}
    <PropRow label="selected" value={sceneSelectedId ?? "none"} />
    <PropRow label="active" value={sceneActiveId ?? "none"} />
  </Section>

  <Section sectionId="demo-treelist-pool" title="TreeList — GPU Buffer Pool" card>
    <div class="demo-note">
      Right-click: Evict, Force Rebuild, Destroy · Live sparklines per buffer ·
      Status dots · Group header BarMeter · Start to simulate age drift ·
      Stale buffers trend red, error buffers fade
    </div>

    <div class="treelist-demo-frame">
      <TreeList
        domain={poolDomain}
        data={poolSlots}
        selectedId={poolSelectedId}
        activeId={poolActiveId}
        onselectionchange={onPoolSelChange}
      />
    </div>

    <div class="btn-row" style="margin-top: 6px;">
      <ActionButton onclick={togglePoolSim}>
        {poolSimulating ? "Stop" : "Start"} Simulation
      </ActionButton>
    </div>
  </Section>

  <Section sectionId="demo-treelist-passes" title="TreeList — Render Passes" card>
    <div class="demo-note">
      Right-click: Solo, Move Up/Down, Inspect Reads/Writes ·
      Toggle passes on/off · Disabled passes fade · Phase totals in headers ·
      4 responsive columns · Reads/Writes hide below 340/400px
    </div>

    <div class="treelist-demo-frame">
      <TreeList
        domain={passDomain}
        data={passNodes}
        selectedId={passSelectedId}
        activeId={passActiveId}
        onselectionchange={onPassSelChange}
      />
    </div>
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

  /* ── TreeList demo frames ────────────────────────────────────────────── */
  .treelist-demo-frame {
    height: 220px;
    border: 1px solid var(--stroke-lo);
    border-radius: 4px;
    overflow: hidden;
    margin-bottom: 6px;
  }
</style>
