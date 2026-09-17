<script lang="ts">
  import Legacy from "./Legacy.svelte";
  let x = "a";
</script>
<Legacy><Legacy class:a={x} slot={x} /></Legacy>
