<script lang="ts">
  import C from './Caccessors_.svelte';
  let c: C;
  const s: string = c.value;
</script>
<C bind:this={c} />
