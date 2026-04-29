<script lang="ts">
    // Round-8 follow-up #3: `{:catch { message }}` declares the
    // destructure leaf `message`. Upstream resolves catch bindings
    // to `__sveltets_2_any({})` regardless of the pattern shape;
    // every leaf types as `any`.
    //
    // Pre-fix native only resolved when `bindings.len() == 1`, so
    // destructure leaves dropped to None and any consumer-side
    // slot-attr referencing them would have been dropped from the
    // emitted slot literal entirely. Post-fix every leaf types as
    // `any` and the slot-attr resolver routes through the standard
    // shadowed-but-resolvable path.
    let { promise }: { promise: Promise<{ ok: true }> } = $props()
</script>

{#await promise}
    loading
{:then value}
    ok: {value.ok}
{:catch { name, message }}
    <slot {name} {message} />
{/await}
