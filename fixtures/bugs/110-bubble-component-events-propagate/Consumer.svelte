<script lang="ts">
    import Wrapper from './Wrapper.svelte'

    // Correct: handler accepts CustomEvent<{id:number}> matching
    // Inner's declared `click` event type. Post-fix Wrapper's
    // surface carries this propagated type; consumer compiles clean.
    function correctHandler(e: CustomEvent<{ id: number }>): void {
        void e.detail.id
    }

    // Wrong: handler declares `(e: number)`, can't accept the
    // CustomEvent. Post-fix this fires TS2345 because Wrapper's
    // `click` event is the propagated CustomEvent<{id:number}> from
    // Inner. Pre-fix the wrapper had no `click` in its surface —
    // consumer's `on:click` typed lax and the wrong handler passed.
    function wrongHandler(e: number): void {
        void e
    }
</script>

<Wrapper on:click={correctHandler} />
<Wrapper on:click={wrongHandler} />
