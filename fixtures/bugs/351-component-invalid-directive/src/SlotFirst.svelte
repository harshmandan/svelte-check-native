<script lang="ts">
  import Legacy from "./Legacy.svelte";
  let x = "a";
</script>
<Legacy><Legacy slot={x} class:a={x} /></Legacy>
