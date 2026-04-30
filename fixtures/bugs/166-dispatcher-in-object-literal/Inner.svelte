<script lang="ts">
    // Round-15 follow-up #2: `collect_function_body_stmts` (the
    // helper backing every dispatcher walker's recursion into
    // nested function bodies) only pierced through Arrow /
    // FunctionExpression / ParenthesizedExpression / CallExpression
    // (callee + args). Common patterns like `const handlers = {
    // save: () => dispatch('save') }` (object-literal value) or
    // `const fns = [() => dispatch('save')]` (array element) hid
    // the inner arrow body from event synthesis and the
    // typed-events rewrite walker.
    //
    // Upstream's TS walker visits every child via
    // `ts.forEachChild`. Native now mirrors that breadth: object
    // literals, arrays, conditionals, logical/binary ops,
    // sequences, assignments, member access, awaits, template
    // literals, TS-cast wrappers, etc. all recurse into their
    // child expressions.
    import { createEventDispatcher } from 'svelte'

    // Typed dispatcher decl hidden inside an object-literal value's
    // arrow body. Pre-fix this typed source was invisible to
    // `find_dispatcher_event_type_sources` — the events surface
    // stayed at the lax `[evt: string]: CustomEvent<any>` default
    // and any consumer handler (right or wrong) passed silently.
    // Post-fix the slice contributes `save: CustomEvent<{id:
    // number}>` to the surface; wrong-typed consumer handlers
    // correctly fail.
    const handlers = {
        init: () => {
            const d = createEventDispatcher<{ save: { id: number } }>()
            void d
        },
    }
    void handlers
</script>

<button>x</button>
