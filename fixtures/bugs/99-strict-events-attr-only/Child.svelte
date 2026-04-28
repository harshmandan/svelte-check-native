<script lang="ts" strictEvents>
    // Reviewer follow-up #2: `<script strictEvents>` is the second of
    // the three event-narrowing triggers (the first is `interface
    // $$Events`, the third is runes mode). Pre-fix the trigger only
    // controlled `narrow_events` (which gates dispatcher detail-map
    // synthesis), but `events_field` in the render-fn return only
    // resolved to `$$Events` when an interface was declared OR a
    // synth alias was emitted. With this Child — strictEvents but
    // no dispatcher, no bubbled DOM events, no interface — the
    // `events_field` fell back to `{ [evt: string]: CustomEvent<any> }`,
    // so consumers' `on:NAME` typed as `(...args: any[]) => any`
    // and any name was accepted.
    //
    // Upstream: `_events(strictEvents, …)` in
    // `language-tools/.../addComponentExport.ts:416-418` drops the
    // `with_any_event` wrapper whenever `strictEvents` is true,
    // regardless of whether events were declared.
</script>

<button>plain button, no on: directive at all</button>
