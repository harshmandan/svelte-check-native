// Tests four overlay shapes for the bits-ui `child` snippet pattern.
// Goal: identify which shape causes TS to LOSE contextual typing on
// `({ props })` (firing TS7031), to MATCH upstream's bench behavior.

import { Trigger } from './trigger.ts';
import type {
    Component,
    ComponentConstructorOptions,
    SvelteComponent,
} from 'svelte';

// Upstream-style ensureComponent — extracts Component<P> via the
// `Component<infer P, ...>` branch and returns a typed constructor.
declare function __sveltets_2_ensureComponent<
    T extends Component<any, any, any> | (new (...args: any[]) => any) | null | undefined,
>(
    type: T,
): NonNullable<
    T extends Component<infer P extends Record<string, any>, infer X, infer B>
        ? new (options: ComponentConstructorOptions<P>) => SvelteComponent<P> & X & {
              $$bindings?: B;
          }
        : T extends new (...args: any[]) => any
          ? T
          : never
>;

declare function __sveltets_2_any(arg?: number): any;

// ---- V1: ours-current — bare arrow, return __svn_snippet_return() ----

declare function __svn_snippet_return(): any;

function v1_ours(): void {
    {
        const C = __sveltets_2_ensureComponent(Trigger);
        new C({
            target: __sveltets_2_any(),
            props: {
                child: ({ props }) => {
                    void props;
                    return __svn_snippet_return();
                },
            },
        });
    }
}

// ---- V2: upstream-shape — body wrapped in `async () => {}; return any(0)` ----

function v2_upstream(): void {
    {
        const C = __sveltets_2_ensureComponent(Trigger);
        const inst = new C({
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
        // Upstream extracts the typed prop-def at the END.
        const { child } = inst.$$prop_def;
        void child;
    }
}

// ---- V3: upstream-shape WITHOUT the prop_def extract ----

function v3_upstream_no_extract(): void {
    {
        const C = __sveltets_2_ensureComponent(Trigger);
        new C({
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
    }
}

// ---- V4: ours-current shape but explicitly NO return ----

function v4_no_return(): void {
    {
        const C = __sveltets_2_ensureComponent(Trigger);
        new C({
            target: __sveltets_2_any(),
            props: {
                child: ({ props }) => {
                    void props;
                },
            },
        });
    }
}

void v1_ours;
void v2_upstream;
void v3_upstream_no_extract;
void v4_no_return;
