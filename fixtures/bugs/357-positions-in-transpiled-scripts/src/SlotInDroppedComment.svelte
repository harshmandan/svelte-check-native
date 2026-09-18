<script lang="ts">
  import type { Snippet } from "svelte";
  let { s }: { s: Snippet } = $props();
  // $$slot doc for the type
  type Count = number;
  let count: Count = 1; count;
  const n = $$slots;
</script>

{@render s()}{n}
