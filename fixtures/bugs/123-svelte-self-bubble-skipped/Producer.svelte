<script lang="ts">
    // Round-7 follow-up #7: `<svelte:self on:click>` is a bare
    // bubble — but upstream's `event-handler.ts:12-15` skips bubble
    // registration when the parent is `<svelte:self>` (bubbling
    // self's own events back into self's `$$Events` is a no-op and
    // wrongly disqualifies the runes fn_component shape). Pre-fix
    // native counted the directive as a bubbled-component event,
    // which set `has_bubbled_component_event` and pushed
    // `bubbled_component_events`; emit then dropped this Producer
    // off the fn-component path onto the iso `$$IsomorphicComponent`
    // shape, breaking `(typeof Producer)[]` consumer patterns. The
    // `$inst.$on("click", () => {})` call still fires so self's
    // declared events are still type-checked.
    let { id }: { id: string } = $props()
    void id
</script>

{#if false}
    <svelte:self {id} on:click />
{/if}
