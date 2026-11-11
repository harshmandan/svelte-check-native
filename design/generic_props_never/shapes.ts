// A generic component's call signature, old shape vs upstream's generic shape.
// `Generic<{ key: 'entter' }>` has props that resolve to `never`.
type Validate<K extends string> = K extends 'enter' | 'escape' ? K : never

declare function $$render<Keys extends { key?: string }>(): {
    props: Keys & { key?: Validate<NonNullable<Keys['key']>> }
    events: {}
    slots: {}
    exports: {}
}
declare class Render<Keys extends { key?: string }> {
    props(): ReturnType<typeof $$render<Keys>>['props']
    events(): ReturnType<typeof $$render<Keys>>['events']
    slots(): ReturnType<typeof $$render<Keys>>['slots']
    exports(): ReturnType<typeof $$render<Keys>>['exports']
}

// Old: the non-generic shim's type-level choice, inlined on a concrete type.
interface OldIso {
    <Keys extends { key?: string }>(
        internal: unknown,
        props: ReturnType<Render<Keys>['props']> extends Record<string, never>
            ? { $$events?: ReturnType<Render<Keys>['events']>; $$slots?: ReturnType<Render<Keys>['slots']> }
            : ReturnType<Render<Keys>['props']> & { $$events?: ReturnType<Render<Keys>['events']>; $$slots?: ReturnType<Render<Keys>['slots']> },
    ): ReturnType<Render<Keys>['exports']> & { $set?: any; $on?: any }
}
// New: upstream's generic shape — the choice is made when emitting (this
// component has typed props, a runes component with no events or slots,
// so the carriers object is empty).
interface NewIso {
    <Keys extends { key?: string }>(internal: unknown, props: ReturnType<Render<Keys>['props']> & {}): ReturnType<Render<Keys>['exports']>
}
declare const OldC: OldIso
declare const NewC: NewIso

type NewInvalid = Parameters<typeof NewC<{ key: 'entter' }>>[1]
export const newIsNever: [NewInvalid['key']] extends [never] ? true : false = true
type NewValid = Parameters<typeof NewC<{ key: 'enter' }>>[1]
export const newValid: NewValid['key'] = 'enter'

// Expected: TS2322 (true not assignable to false) and TS2339 (no `key`).
type OldInvalid = Parameters<typeof OldC<{ key: 'entter' }>>[1]
export const oldIsNever: [OldInvalid['key']] extends [never] ? true : false = true
