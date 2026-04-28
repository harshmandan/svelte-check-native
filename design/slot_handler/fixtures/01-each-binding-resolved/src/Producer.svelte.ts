// Hand-written overlay for what the producer emit SHOULD produce
// for a Child component containing:
//
//   <script lang="ts">
//     let { items }: { items: { id: number; label: string }[] } = $props();
//   </script>
//   {#each items as item, i}
//       <slot item={item} index={i} />
//   {/each}
//
// The slot's `item` attr's source identifier (`item`) is shadowed by
// the each-block's context binding — pre-implementation, our walker
// SKIPS such attrs (so the slot-def is empty `{}`). Post-implementation,
// the resolver projects:
//
//   - `item` → `(typeof items extends ReadonlyArray<infer __svn_T> ? __svn_T : never)`
//   - `i` → `number`
//
// The projection lives in TYPE position inside the `slots:` field of
// `$$render`'s return literal. Consumer side then reads it via
// `Awaited<ReturnType<typeof $$render>>['slots']` and projects per slot
// name + per attr name into the consumer's `<Child let:item let:index>`
// destructure.
//
// `Iterable<infer T>` (not `ReadonlyArray<infer T>`) so the projection
// also covers `{#each set as item}` / `{#each map.entries() as [k, v]}`
// (Set / Map / generator iterables). Arrays already implement
// `Iterable<T>`, so the common case lines up unchanged.

async function $$render() {
    const items = __svn_any<{ id: number; label: string }[]>();
    void items;
    return {
        props: undefined as any as { items: { id: number; label: string }[] },
        events: undefined as any as { [evt: string]: CustomEvent<any> },
        slots: undefined as any as {
            'default': {
                item: (typeof items extends Iterable<infer __svn_T> ? __svn_T : never),
                index: number,
            };
        },
        bindings: undefined as any as string,
        exports: undefined as any as {},
    };
}
$$render;

// Consumer-side projection. The `Awaited<…>['slots']['default']`
// surfaces are { item, index } where item carries the projected
// element type. Member access on the resolved item is unwrapped:
type SlotsType = Awaited<ReturnType<typeof $$render>>['slots'];
type DefaultSlot = SlotsType['default'];

// Clean: every member access lines up with the source declaration.
function consumerClean(s: DefaultSlot) {
    void s.item.id;     // number
    void s.item.label;  // string
    void s.index;       // number
}
void consumerClean;

// Break case: typo'd member access on the resolved item. Tsgo MUST
// fire TS2339 here — that's the failure mode this fixture validates.
// Under pre-implementation `slot-def is empty {}`, the typo would
// pass silently because `s.item` would be `any`.
function consumerBroken(s: DefaultSlot) {
    // @ts-expect-error TS2339: Property 'noSuchField' does not exist
    void s.item.noSuchField;
}
void consumerBroken;
