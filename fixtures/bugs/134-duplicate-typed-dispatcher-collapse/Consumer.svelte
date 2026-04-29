<script lang="ts">
    import Inner from './Inner.svelte'

    // `hi` collapses to `CustomEvent<any>` — a CustomEvent handler
    // accepting any detail passes. Pre-fix the type was
    // `CustomEvent<boolean & string>` = `CustomEvent<never>`, which
    // TS rejects against every consumer handler.
    function handleAny(e: CustomEvent): void {
        void e.detail
    }
</script>

<Inner on:hi={handleAny} />
