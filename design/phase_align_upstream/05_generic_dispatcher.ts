// Parent instantiates generic dispatcher with concrete `string`.
// `on:change` handler's `e.detail.value` must narrow to `string` —
// NOT widen to `string | number` (the $$Generic constraint).
//
// This is the fixture Item 5 would have failed. If Phase A passes
// THIS file with the assertion below compiling, the overlay shape
// works — and the implementation has a chance.

/// <reference path="./__shims.d.ts" />
import GenericDispatcher from './GenericDispatcher.overlay';

function $$render_parent() {
    async function __svn_tpl_check() {
        {
            const __svn_C_1 = __svn_ensure_component(GenericDispatcher<string>);
            const __svn_inst_1 = new __svn_C_1({
                target: __svn_any(),
                props: {},
            });
            __svn_inst_1.$on('change', (e) => {
                // e.detail.value should narrow to `string` here.
                // If Item 5's failure mode repeats, this would widen
                // to `string | number` and the `.length` access would
                // still work but `toUpperCase()` wouldn't. We assert
                // the exact narrow via a structural type check.
                const narrowed: string = e.detail.value;
                void narrowed;
                e.detail.value.toUpperCase();
                e.detail.nativeEvent.preventDefault();
            });
        }
    }
    void __svn_tpl_check;
}
void $$render_parent;
