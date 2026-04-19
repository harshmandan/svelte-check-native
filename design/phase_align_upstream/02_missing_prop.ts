// Missing required prop → TS2741 expected at the `new ... ({...})` site.
// Upstream reports the error at the `Jsdoc` name position (col 1 of `<Jsdoc />`);
// our fixture here is overlay-level, so the error will fire at the `{}` literal.
// Source-map handling is Phase C's job.

/// <reference path="./__shims.d.ts" />
import Jsdoc from './Jsdoc.overlay';

function $$render_parent() {
    async function __svn_tpl_check() {
        {
            const __svn_C_1 = __svn_ensure_component(Jsdoc);
            new __svn_C_1({ target: __svn_any(), props: {} });
        }
    }
    void __svn_tpl_check;
}
void $$render_parent;
