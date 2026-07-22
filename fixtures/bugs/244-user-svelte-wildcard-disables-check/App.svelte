<script lang="ts">
  // `./Missing.svelte` does not exist. But this workspace declares its own
  // `declare module '*.svelte'` wildcard (svelte-shim.d.ts) — a
  // USER-authored one, not svelte's. The default `svelte-check` strips
  // svelte's own wildcard but KEEPS a user's, so it resolves this import
  // and reports nothing. Our TSGO-ENHANCEMENT guard mirrors that: seeing a
  // workspace `*.svelte` wildcard it disables the missing-import check
  // entirely, so the result stays clean (tsgo's wildcard also swallows it).
  import Missing from './Missing.svelte'
</script>

<Missing />
