<script lang="ts">
    import Inner from './Inner.svelte'

    // `hi` resolves to EventsB's typing (`string`) — last source
    // wins per upstream's spread semantics.
    function handleString(e: CustomEvent<string>): void {
        void e.detail
    }

    // `foo` only declared in EventsA → `CustomEvent<number>`. Passes.
    function handleNumber(e: CustomEvent<number>): void {
        void e.detail
    }

    // `hi` is now `CustomEvent<string>`, NOT `CustomEvent<boolean>`
    // (which was EventsA's earlier declaration). Pre-fix `boolean &
    // string` = `never` made this and EVERY handler fail.
    function handleBoolean(e: CustomEvent<boolean>): void {
        void e.detail
    }
</script>

<Inner on:hi={handleString} />
<Inner on:foo={handleNumber} />
<!-- Wrong shape: hi is string, not boolean. Must fire TS2345. -->
<Inner on:hi={handleBoolean} />
