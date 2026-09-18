<script lang="ts">
  import type { Snippet } from "svelte";
  let { s }: { s: Snippet } = $props();
  let size: "$$slot" | number = 1; size;
  const n = $$slots;
</script>

{@render s()}{n}
