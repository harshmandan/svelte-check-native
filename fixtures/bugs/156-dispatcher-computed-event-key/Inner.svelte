<script lang="ts">
    // Round-14 follow-up #6: an inline typed dispatcher whose member
    // KEY is a computed `[EVENT]` where `EVENT` is a top-level
    // `const EVENT = 'literal'` resolves at synth time. Combined with
    // a second typed dispatcher that declares the same event name as
    // a static key, the duplicate-collapse path should fire — the
    // shared name routes through the untyped layer and emerges as
    // `CustomEvent<any>` rather than the type-level intersection
    // (`boolean & string` = `never`).
    //
    // Upstream resolves this in `ComponentEvents.ts:319` (`getName`):
    // when a property name is a computed identifier, it walks
    // top-level decls via `getVariableAtTopLevel` and substitutes
    // the string-literal initializer's text. Pre-fix native only
    // accepted `StaticIdentifier` and `StringLiteral` keys, so the
    // computed name was silently dropped from the duplicate-detection
    // pass and the two typed sources intersected to `never`.
    import { createEventDispatcher } from 'svelte'

    const EVENT = 'hi'

    const _d1 = createEventDispatcher<{ [EVENT]: boolean }>()
    const _d2 = createEventDispatcher<{ hi: string }>()
    void _d1
    void _d2
</script>

<button>x</button>
