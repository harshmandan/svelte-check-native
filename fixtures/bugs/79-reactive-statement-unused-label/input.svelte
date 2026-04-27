<script lang="ts">
    // Svelte-4 reactive `$:` statement that's not a binary assignment.
    // Both ours and upstream emit this as `;() => { $: <expr> }` so
    // tsgo type-checks the expression body without actually executing
    // it. Under `allowUnusedLabels: false` (default in many strict-mode
    // tsconfigs, including SvelteKit + threlte), the structural `$:`
    // label fires TS7028 on every reactive statement.
    //
    // Upstream filters TS7028 diagnostics that target a `$` identifier
    // on a reactive label (DiagnosticsProvider.ts:476-495). Ours now
    // does the same in `crates/typecheck/src/lib.rs::map_diagnostic`.

    const obj: { setup: (cfg: { audio: boolean }) => void } = {
        setup: () => {},
    };
    let audio = $state(true);

    $: obj.setup({ audio });

    $: console.log('reactive side-effect', audio);
</script>

<p>{audio}</p>
