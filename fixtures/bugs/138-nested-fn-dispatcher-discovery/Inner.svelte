<script lang="ts">
    // Round-10 follow-up #2: a typed dispatcher declared inside an
    // arrow expression assigned to a variable. Pre-fix native's
    // `statement_collect_typed_dispatcher_slices` recursed through
    // FunctionDeclaration / Block / If statements but NOT into
    // ArrowFunctionExpression / FunctionExpression bodies attached
    // as variable initializers. So this Inner's typed dispatcher
    // was missed → `synthesized_typed_events` was None →
    // `events_alias_body` was None → consumer-side `<Inner on:foo>`
    // resolved through the lax untyped index-signature path,
    // accepting any handler shape silently.
    //
    // Upstream's TS walker visits nested variable declarations
    // wherever they appear (including inside arrow/function
    // expression bodies). Native now mirrors via the
    // `statements_inside_function_expr` helper.
    import { createEventDispatcher } from 'svelte'

    const setup = () => {
        const _dispatch = createEventDispatcher<{ foo: string }>()
        void _dispatch
    }
    void setup
</script>

<button>x</button>
