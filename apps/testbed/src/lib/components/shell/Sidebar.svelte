<script lang="ts">
  import { Layers, Database, Activity, Gauge, Bug, Settings, Palette } from "lucide-svelte";

  let {
    activePanel,
    onTabClick,
  }: {
    activePanel: string;
    onTabClick: (value: string) => void;
  } = $props();

  const top = [
    { value: "scene",    Icon: Layers,   label: "Scene" },
    { value: "pool",     Icon: Database, label: "GPU Pool" },
    { value: "proto",    Icon: Activity, label: "Edit Protocol" },
    { value: "perf",     Icon: Gauge,    label: "Performance" },
    { value: "debug",    Icon: Bug,      label: "Debug" },
  ];

  const bottom = [
    { value: "demo",     Icon: Palette,  label: "Component Demo" },
    { value: "settings", Icon: Settings, label: "Settings" },
  ];
</script>

<nav class="sidebar">
  <div class="sidebar-group">
    {#each top as { value, Icon, label }}
      <button
        class="sidebar-item"
        data-state={activePanel === value ? "active" : "inactive"}
        title={label}
        onclick={() => onTabClick(value)}
      >
        <Icon size={18} />
      </button>
    {/each}
  </div>

  <div class="sidebar-group">
    {#each bottom as { value, Icon, label }}
      <button
        class="sidebar-item"
        data-state={activePanel === value ? "active" : "inactive"}
        title={label}
        onclick={() => onTabClick(value)}
      >
        <Icon size={18} />
      </button>
    {/each}
  </div>
</nav>

<style>
  .sidebar-group {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
</style>
