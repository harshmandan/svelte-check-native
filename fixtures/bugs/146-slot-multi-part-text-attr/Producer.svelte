<script lang="ts">
    // Round-12 follow-up #5: multi-part text attrs on `<slot>`
    // (e.g. `label="hi {name}"`) — pre-fix native dropped these
    // attrs entirely from the slot literal, so consumer-side
    // `let:label` typed as `any` and any field/argument check
    // passed silently.
    //
    // Upstream's `slot.ts:46` casts any multi-part interpolated
    // attr to a dummy string expression at value level, giving
    // `label: string` at the slot prop type. Native now mirrors
    // by emitting `label` as a `Resolved(Type("string"))` slot
    // attr.
    let { name }: { name: string } = $props()
    void name
</script>

<slot label="hi {name}" />
