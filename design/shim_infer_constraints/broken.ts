// Companion to clean.ts: misspelled event name should fire 2339.
declare class SvelteComponent<P = any, E extends Record<string, any> = any, S extends Record<string, any> = any> {
    $$events_def: E;
    $$slot_def: S;
}

type __SvnComponentEvents<C> = C extends { readonly __svn_events: infer E }
    ? E
    : C extends new (...args: any[]) => SvelteComponent<any, infer E extends Record<string, any>, any>
        ? E
        : Record<string, any>;

declare class StrictEvents extends SvelteComponent<{}, { foo: CustomEvent<string> }, {}> {}

type E = __SvnComponentEvents<typeof StrictEvents>;
const e: E = {} as any;
e.bar;   // expect 2339: Property 'bar' does not exist on type '{ foo: CustomEvent<string>; }'
