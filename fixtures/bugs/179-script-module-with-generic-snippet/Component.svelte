<script lang="ts" module></script>

<script lang="ts" generics="T">
    // language-tools#2917 — when `<script module>` coexists with
    // `<script generics="T">` and a `{#snippet defaultSnip(generic: T)}`
    // references the generic, upstream svelte2tsx fires TS2304 'Cannot
    // find name T' because the module-script emit pulls the
    // generic-binder out of scope for the snippet body. Our
    // `__svn_Render_<hash><T> { ... }` class-wrapper pattern (CLAUDE.md
    // rule #7) keeps T in scope across both surfaces, so this resolves
    // cleanly.
    import type { Snippet } from 'svelte'
    let { snip = defaultSnip }: { snip: Snippet<[T]> } = $props()
    void snip
</script>

{#snippet defaultSnip(generic: T)}
    {generic}
{/snippet}
