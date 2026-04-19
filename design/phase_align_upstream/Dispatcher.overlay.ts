// Simulated overlay for a child that uses typed createEventDispatcher:
//   <script>
//     const dispatch = createEventDispatcher<{foo: string}>()
//   </script>

/// <reference path="./__shims.d.ts" />

function $$render_dispatcher() {
    const dispatch = createEventDispatcher<{ foo: string }>();
    void dispatch;
    return {
        props: {},
        events: {
            ...__svn_to_event_typings<{ foo: string }>(),
        },
        slots: {},
    };
}
void $$render_dispatcher;

class __svn_Render_dispatcher {
    props() {
        return $$render_dispatcher().props;
    }
    events() {
        return $$render_dispatcher().events;
    }
    slots() {
        return $$render_dispatcher().slots;
    }
}

class Dispatcher__SvelteComponent_ {
    $$prop_def!: ReturnType<__svn_Render_dispatcher['props']>;
    $$events_def!: ReturnType<__svn_Render_dispatcher['events']>;
    constructor(_: {
        target: any;
        props: ReturnType<__svn_Render_dispatcher['props']>;
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

export default Dispatcher__SvelteComponent_;
