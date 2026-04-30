<script lang="ts">
    import Inner from './Inner.svelte'

    // Post-fix: `save` is `CustomEvent<{id: number}>`. A handler
    // typed `(e: CustomEvent<string>) => void` is NOT assignable
    // — TS2345 fires. Pre-fix the typed dispatcher hidden in the
    // object-literal value was invisible to native's walkers; the
    // events surface stayed at the lax default and the wrong-typed
    // handler was accepted silently.
    function handleStr(e: CustomEvent<string>): void {
        void e.detail
    }
</script>

<Inner on:save={handleStr} />
