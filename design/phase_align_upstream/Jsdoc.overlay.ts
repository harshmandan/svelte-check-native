// Simulated overlay for `Jsdoc.svelte`:
//   <script>export let b: string</script>
// After Phase C, our emit would produce something shaped like this.
// Mirrors upstream's __sveltets_Render<T> + SvelteComponentTyped pattern.

/// <reference path="./__shims.d.ts" />

function $$render_jsdoc() {
    let b: string = '' as any;
    void b;
    return { props: { b }, events: {}, slots: {} };
}
void $$render_jsdoc;

class __svn_Render_jsdoc {
    props() {
        return $$render_jsdoc().props;
    }
    events() {
        return $$render_jsdoc().events;
    }
    slots() {
        return $$render_jsdoc().slots;
    }
}

// Minimal stand-in for `SvelteComponentTyped<Props, Events, Slots>` —
// we only need a typed constructor for the parent's new-site check.
class Jsdoc__SvelteComponent_ {
    $$prop_def!: ReturnType<__svn_Render_jsdoc['props']>;
    $$events_def!: ReturnType<__svn_Render_jsdoc['events']>;
    constructor(_: {
        target: any;
        props: ReturnType<__svn_Render_jsdoc['props']>;
    }) {}
    $on<K extends keyof this['$$events_def'] & string>(
        type: K,
        handler: (
            e: this['$$events_def'][K] extends CustomEvent<any>
                ? this['$$events_def'][K]
                : never,
        ) => any,
    ): () => void {
        void type;
        void handler;
        return () => {};
    }
}

export default Jsdoc__SvelteComponent_;
