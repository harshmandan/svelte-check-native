<script lang="ts">
  import C from './Caccessors__false__.svelte';
  let c: C;
  const s: string = c.value;
</script>
<C bind:this={c} />
