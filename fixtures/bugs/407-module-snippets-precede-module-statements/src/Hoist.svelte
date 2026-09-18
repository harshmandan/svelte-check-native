<script module lang="ts">
    import type { Snippet } from "svelte";
    export const foo: Snippet = _foo;
</script>

{#snippet _foo()}<p>hi</p>{/snippet}
