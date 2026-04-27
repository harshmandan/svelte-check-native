// Models the layerchart `Svg.svelte` overlay (no-Props component).
// This is the case the v0.5+1 commit fd126e98 was added to handle —
// `let Context: Component = Svg;` user code in docs.
//
// Three shape variants — same as instance_*.ts but with empty Props.

// ---- Variant A: OURS — iso WITH `& { $set?, $on? }` (current behavior) ----

interface SvgPropsA {}

interface $$IsoSvgOurs {
    new (
        options: import('svelte').ComponentConstructorOptions<SvgPropsA>,
    ): import('svelte').SvelteComponent<SvgPropsA, {}, {}> & { $$bindings?: '' } & {};
    (internal: unknown, props: SvgPropsA): {} & { $set?: any; $on?: any };
    z_$$bindings?: '';
}

export const SvgOurs: $$IsoSvgOurs = null as any;

// ---- Variant B: UPSTREAM per-component iso — NO `& { $set?, $on? }` ----

interface SvgPropsB {}

interface $$IsoSvgUp {
    new (
        options: import('svelte').ComponentConstructorOptions<SvgPropsB>,
    ): import('svelte').SvelteComponent<SvgPropsB, {}, {}> & { $$bindings?: '' } & {};
    (internal: unknown, props: SvgPropsB & {}): {};
    z_$$bindings?: '';
}

export const SvgUp: $$IsoSvgUp = null as any;

// ---- Variant C: Component<> — what __sveltets_2_fn_component returns ----

export const SvgComponent: import('svelte').Component<{}, {}, ''> = null as any;
