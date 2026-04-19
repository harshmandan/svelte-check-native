// Parent attaches `on:foo={(e: CustomEvent<number>) => …}` — handler's
// declared event payload is `number` but dispatcher says `string`.
// Expected: TS2345 (or TS2322 depending on which slot tsgo flags).

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
            const handler = (e: CustomEvent<number>) => {
                e.detail.toFixed(0);
            };
            __svn_inst_1.$on('foo', handler);
        }
    }
    void __svn_tpl_check;
}
void $$render_parent;
