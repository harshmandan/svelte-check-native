// Parent attaches `on:foo={handler}` — handler's param narrowed to
// `CustomEvent<string>`. `e.detail.length` must type-check (string has .length).
// Expected: 0 diagnostics.

/// <reference path="./__shims.d.ts" />
import Dispatcher from './Dispatcher.overlay';

function $$render_parent() {
    async function __svn_tpl_check() {
        {
            const __svn_C_1 = __svn_ensure_component(Dispatcher);
            const __svn_inst_1 = new __svn_C_1({
                target: __svn_any(),
                props: {},
            });
            __svn_inst_1.$on('foo', (e) => {
                // e.detail should be narrowed to `string` here.
                e.detail.length;
            });
        }
    }
    void __svn_tpl_check;
}
void $$render_parent;
