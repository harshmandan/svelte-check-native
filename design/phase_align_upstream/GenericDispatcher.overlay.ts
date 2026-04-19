// Simulated overlay for a generic-typed dispatcher child:
//   <script lang="ts">
//     type ValueType = $$Generic<string | number>
//     const dispatch = createEventDispatcher<{
//       change: { value: ValueType; nativeEvent: Event }
//     }>()
//   </script>
//
// Key: ValueType is hoisted as a GENERIC on the render function AND
// the __svn_Render class. Upstream's pattern: generics are class-bound,
// so ReturnType<Render<T>['events']> resolves T correctly at the parent's
// `new Child<concrete>()` site.
//
// Contrast: Item 5 synthesized `type $$Events = {change:{value:ValueType}}`
// at module scope where ValueType wasn't visible. The $$Generic alias
// was left in body (declared inside the render function), and module-scope
// $$Events fell back to the string|number constraint widening. 18 FPs.

/// <reference path="./__shims.d.ts" />

function $$render_gdispatcher<ValueType extends string | number>() {
    const dispatch = createEventDispatcher<{
        change: { value: ValueType; nativeEvent: Event };
    }>();
    void dispatch;
    return {
        props: {},
        events: {
            ...__svn_to_event_typings<{
                change: { value: ValueType; nativeEvent: Event };
            }>(),
        },
        slots: {},
    };
}
void $$render_gdispatcher;

class __svn_Render_gdispatcher<ValueType extends string | number> {
    props() {
        return $$render_gdispatcher<ValueType>().props;
    }
    events() {
        return $$render_gdispatcher<ValueType>().events;
    }
    slots() {
        return $$render_gdispatcher<ValueType>().slots;
    }
}

class GenericDispatcher__SvelteComponent_<
    ValueType extends string | number = string | number,
> {
    $$prop_def!: ReturnType<__svn_Render_gdispatcher<ValueType>['props']>;
    $$events_def!: ReturnType<__svn_Render_gdispatcher<ValueType>['events']>;
    constructor(_: {
        target: any;
        props: ReturnType<__svn_Render_gdispatcher<ValueType>['props']>;
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

export default GenericDispatcher__SvelteComponent_;
