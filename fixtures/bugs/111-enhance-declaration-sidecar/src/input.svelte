<script lang="ts">
    // No Generated.svelte exists — the hand-written .d.svelte.ts
    // declaration satisfies the import, which is what
    // allowArbitraryExtensions is for.
    import Generated from './Generated.svelte';
    // Same mechanism through a paths alias: the sidecar must be found
    // wherever the compiler's resolution lands, not only when the
    // specifier is ./-relative.
    import AliasedGen from '$gen/Gen.svelte';
</script>

<p>{typeof Generated} {typeof AliasedGen}</p>
