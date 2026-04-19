// Shim types the overlay depends on. Mirrors upstream's
// __sveltets_2_* family, adapted for our naming.

// Passthrough — preserves the real constructor signature.
// Mirror of upstream's __sveltets_2_ensureComponent
// (language-tools/packages/svelte2tsx/svelte-shims.d.ts:303).
declare function __svn_ensure_component<T>(c: T): NonNullable<T>;

declare function __svn_any(): any;

// Mirror of upstream's __sveltets_2_toEventTypings
// (svelte-shims.d.ts:203). Turns a dispatcher generic
// `{foo: string}` into `{foo: CustomEvent<string>}`.
declare function __svn_to_event_typings<Typings>(): {
    [K in keyof Typings]: CustomEvent<Typings[K]>;
};

declare function createEventDispatcher<
    EventMap extends Record<string, any> = any,
>(): <EventKey extends keyof EventMap & string>(
    type: EventKey,
    detail?: EventMap[EventKey],
) => boolean;
