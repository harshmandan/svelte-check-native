<script lang="ts">
  import Legacy from "./Legacy.svelte";
  const k = 1;
</script>
<Legacy bind:value={k} class:a={k} />
