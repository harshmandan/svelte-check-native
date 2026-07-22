<script lang="ts">
  // `./Missing.svelte` does not exist. tsgo can't flag it — the
  // `declare module '*.svelte'` wildcard in `svelte-shim.d.ts` (which
  // stands in for svelte's own `types/index.d.ts`) resolves it to `any`.
  // Our native check reports the TS2307 that the default `svelte-check`
  // would, keeping us at parity with it rather than `--tsgo`.
  import Missing from './Missing.svelte'
</script>

<Missing />
