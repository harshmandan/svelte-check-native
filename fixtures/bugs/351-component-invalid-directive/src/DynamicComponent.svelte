<script lang="ts">
  import C from "./C.svelte";
  let x = $state("red");
</script>
<svelte:component this={C} style:color={x} />
{x}
