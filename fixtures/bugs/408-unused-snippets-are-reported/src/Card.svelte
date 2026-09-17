<script lang="ts">
  import type { Snippet } from "svelte";
  let { header, children }: { header: Snippet; children: Snippet } = $props();
</script>
{@render header()}{@render children()}
