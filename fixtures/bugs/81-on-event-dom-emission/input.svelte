<script lang="ts">
    // Pre-fix: our DOM-element emission silently dropped Svelte-4
    // `on:event={fn}` directives, so reassignments inside the user's
    // handler were invisible to TS's flow analysis — `processor` got
    // narrowed to its initial literal `"stripe"` and `processor ===
    // "liberapay"` then fired TS2367 "no overlap." Now we emit
    // `"on:click": (handler)` as a key in the createElement attribute
    // literal (matches upstream svelte2tsx's EventHandler.ts DOM
    // branch), so TS sees the assignment and de-narrows.

    type Processor = 'stripe' | 'liberapay';
    let processor: Processor = $state('stripe');
</script>

<button on:click={() => (processor = 'stripe')} class:selected={processor === 'stripe'}>
    Stripe
</button>
<button on:click={() => (processor = 'liberapay')} class:selected={processor === 'liberapay'}>
    Liberapay
</button>

<!-- The forward-event idiom: handler + bare on:NAME for forwarding —
     produces duplicate `"on:click"` keys in the createElement
     literal. Both ours and upstream emit duplicates; upstream filters
     TS1117 ("multiple props same name") on element attribute names;
     we do the same in `crates/typecheck/src/lib.rs::map_diagnostic`. -->
<button on:click={() => (processor = 'stripe')} on:click>
    Forward
</button>
