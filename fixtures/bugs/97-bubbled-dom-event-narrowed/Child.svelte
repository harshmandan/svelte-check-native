<svelte:options runes />
<script lang="ts">
    // Reviewer item #3c part 2: bare `<button on:click>` directives
    // forward the native DOM event up to a parent listener at runtime.
    // At type-check time, the consumer's `<Child on:click={cb}>` should
    // see the DOM event shape (`MouseEvent`) — NOT the lax
    // `CustomEvent<any>` fallback that pre-fix landed.
    //
    // Runes mode is the narrow-trigger here. The walker collects the
    // bare `on:click` on the `<button>` element, emit projects it into
    // `{ "click": HTMLElementEventMap["click"] }`, and the synthesised
    // `$$Events` alias intersects that with the dispatcher detail map
    // (empty in this fixture — there's no dispatcher). The default
    // export's `__svn_events: $$Events` marker then flows the typed
    // map to the consumer through `__svn_ensure_component`'s typed
    // overload.
    let { label = '' }: { label?: string } = $props()
</script>

<button on:click>{label}</button>
