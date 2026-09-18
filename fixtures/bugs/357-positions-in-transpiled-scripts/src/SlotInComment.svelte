<script lang="ts">
  import type { Snippet } from "svelte";
  let { s }: { s: Snippet } = $props();
  // $$slot mentioned first
  const n = $$slots;
</script>

{@render s()}{n}
