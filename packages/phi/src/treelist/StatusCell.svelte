<script lang="ts">
  import StatusIndicator from "../StatusIndicator.svelte";

  let {
    status = "idle",
    label,
  }: {
    status?: "ok" | "warning" | "error" | "idle";
    label?: string;
  } = $props();
</script>

<div class="status-cell">
  <StatusIndicator {status} {label} />
</div>

<style>
  .status-cell {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
  }
</style>
