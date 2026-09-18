<script lang="ts">
  import C from "./C.svelte";
  let x = $state(true);
</script>
<C class:a={x} />
{x}
