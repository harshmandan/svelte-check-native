// Minimal repro: copies one line of upstream's failing overlay verbatim,
// with stub imports.
//
// EXPECTED: TS7031 fires on `({ props })`.

import type { Snippet, Component } from 'svelte';

// Stub imports that match what the real overlay would resolve to.
// `Drawer` is a `* as` namespace — its `Trigger` is `Component<...>`.

interface TriggerProps {
    child?: Snippet<[{ props: Record<string, unknown> }]>;
    children?: Snippet;
    disabled?: boolean;
}

const _Trigger: Component<TriggerProps, {}, ''> = null as any;

declare const Drawer: {
    Trigger: typeof _Trigger;
    Root: any;
};

// Mirror upstream's EXACT shim signature including the
// `ConstructorOfATypedSvelteComponent` arm (which is the FIRST
// overload arm — TS picks it eagerly when applicable).
type ATypedSvelteComponent = {
    $$prop_def: any;
    $$events_def: any;
    $$slot_def: any;
    $on(event: string, handler: any): () => void;
};
type ConstructorOfATypedSvelteComponent = new (args: {
    target: any;
    props?: any;
}) => ATypedSvelteComponent;

declare function __sveltets_2_ensureComponent<
    T extends
        | ConstructorOfATypedSvelteComponent
        | Component<any, any, any>
        | null
        | undefined,
>(
    type: T,
): NonNullable<
    T extends ConstructorOfATypedSvelteComponent
        ? T
        : T extends Component<
                infer Props extends Record<string, any>,
                infer Exports extends Record<string, any>,
                infer Bindings extends string
            >
          ? new (options: { target: any; props?: Props }) => {
                $$prop_def: Props;
                $$events_def: any;
                $$slot_def: any;
            } & Exports & { $$bindings: Bindings }
          : never
>;

declare function __sveltets_2_any(arg?: number): any;

// Verbatim line 69 of upstream's chart-code-viewer overlay.
async () => {
    {
        const $$_reggirT_rewarD1C = __sveltets_2_ensureComponent(Drawer.Trigger);
        const $$_reggirT_rewarD1 = new $$_reggirT_rewarD1C({
            target: __sveltets_2_any(),
            props: {
                child: ({ props }) => {
                    async () => {
                        void props;
                    };
                    return __sveltets_2_any(0);
                },
            },
        });
        const { child } = $$_reggirT_rewarD1.$$prop_def;
        void child;
    }
};
