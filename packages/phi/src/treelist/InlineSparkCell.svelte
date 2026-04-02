<script lang="ts">
  import Sparkline from "../Sparkline.svelte";

  let {
    values = [],
    warn,
    danger,
  }: {
    values?: number[];
    warn?: number;
    danger?: number;
  } = $props();
</script>

<div class="spark-cell">
  <Sparkline {values} height={14} {warn} {danger} />
</div>

<style>
  .spark-cell {
    width: 100%;
  }
</style>
