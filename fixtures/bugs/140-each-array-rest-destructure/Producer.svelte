<script lang="ts">
    // Round-10 follow-up #4: `{#each tuples as [head, ...tail]}` —
    // `tail` should be the tuple-tail extraction. For
    // `[number, string, boolean]` rows, `tail` is `[string, boolean]`.
    // Pre-fix native treated `tail` as parent path inheritance →
    // typed as the WHOLE element tuple, so `tail[0]` would have
    // wrongly typed as `number` (the head's type) instead of
    // `string` (the tail's first).
    //
    // Upstream's IIFE `((${pattern}) => ${tail})(unwrapArr(rows))`
    // preserves JS rest semantics. Native now mirrors with a TS-
    // level conditional: `T extends readonly [unknown, ...infer R]
    // ? R : never` (with `skip` `unknown` placeholders for elements
    // before the rest).
    let { rows }: { rows: [number, string, boolean][] } = $props()
</script>

{#each rows as [head, ...tail]}
    <slot {head} {tail} />
{/each}
