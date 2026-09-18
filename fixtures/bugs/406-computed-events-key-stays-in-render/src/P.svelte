<script lang="ts">
  import E from "./E.svelte";
</script>
<E on:go={(e) => e.detail.toFixed()} />
