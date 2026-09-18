<script lang="ts">
  import type { Snippet } from "svelte";
  let { s }: { s: Snippet } = $props();
  const label = 'the $$slot text'; label;
  const n = $$slots;
</script>

{@render s()}{n}
