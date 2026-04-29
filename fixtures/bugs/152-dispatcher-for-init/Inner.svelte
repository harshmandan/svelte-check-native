<script lang="ts">
    // Round-13 follow-up #6: dispatcher declaration in a `for`
    // loop's INITIALIZER. Round-12 #4 added For/While/Switch/Try
    // recursion but only walked the bodies — the headers
    // (init/test/update for For; right for ForOf/ForIn;
    // discriminant for Switch; test for While) were skipped, so
    // a dispatcher declared in `for (let d = createEventDispatcher
    // <{ tick: number }>(); cond; ...)` never reached the events
    // surface.
    //
    // Upstream's TS walker visits every part of the for statement
    // via `ts.forEachChild`. Native now mirrors.
    import { createEventDispatcher } from 'svelte'

    function init() {
        for (
            let _d = createEventDispatcher<{ tick: number }>(), i = 0;
            i < 1;
            i++
        ) {
            void _d
        }
    }
    void init
</script>

<button>x</button>
