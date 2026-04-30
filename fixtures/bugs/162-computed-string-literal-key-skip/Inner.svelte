<script lang="ts">
    // Round-15 follow-up #5: typed-dispatcher member keys of the
    // form `['foo']: T` (computed StringLiteral) are NOT a valid
    // event key path upstream — `getName` (`ComponentEvents.ts:319`)
    // matches only `Identifier`, `StringLiteral` (non-computed),
    // and `ComputedPropertyName(Identifier)`-resolved-via-
    // `getIdentifierValue`; everything else throws. Native can't
    // propagate user-script syntax errors so we silently skip the
    // unsupported computed form.
    //
    // Pre-fix native's `expression_collect_inline_typed_members`
    // accepted computed StringLiteral as if it were the equivalent
    // non-computed form, producing a phantom event name that
    // upstream doesn't see. The duplicate-collapse pass then
    // erroneously detected a collision when another typed
    // dispatcher declared the same name in non-computed form.
    //
    // This fixture pairs `<{ ['save']: boolean }>` with
    // `<{ save: string }>`. Pre-fix the duplicate-collapse path
    // saw `save` twice and overrode the event to
    // `CustomEvent<any>`. Post-fix the computed form is skipped,
    // and the event surface keeps the second typed source's
    // `CustomEvent<string>` (the chained-Omit/spread shape gives
    // the second source priority). A consumer handler typed
    // `(e: CustomEvent<number>) => …` then correctly fails
    // TS2322 on the `on:save=` attribute.
    import { createEventDispatcher } from 'svelte'
    const _d1 = createEventDispatcher<{ ['save']: boolean }>()
    const _d2 = createEventDispatcher<{ save: string }>()
    void _d1
    void _d2
</script>

<button>x</button>
