<script lang="ts">
    // Stage 3: slot attrs that aren't bare identifiers — like
    // `<slot label={item.label}>` — used to be silently dropped
    // because the resolver only handled bare-name expressions.
    // Stage 3's OXC rewriter walks the parsed expression and
    // rewrites member access from `.foo` to `['foo']` for
    // type-level access, threading the resolved root through.
    export let rows: { id: string; label: string; meta: { kind: string } }[] = []
</script>

{#each rows as item}
    <slot label={item.label} kind={item.meta.kind} />
{/each}
