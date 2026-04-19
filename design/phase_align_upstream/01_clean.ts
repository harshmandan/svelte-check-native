// Clean case: required prop `b: string` provided → 0 diagnostics expected.

/// <reference path="./__shims.d.ts" />
import Jsdoc from './Jsdoc.overlay';

function $$render_parent() {
    async function __svn_tpl_check() {
        {
            const __svn_C_1 = __svn_ensure_component(Jsdoc);
            new __svn_C_1({ target: __svn_any(), props: { b: 'hi' } });
        }
    }
    void __svn_tpl_check;
}
void $$render_parent;
