// Broken: `style:transform={scale}` where `scale` is typed `unknown`
// (simulated via an explicit annotation — in real code this comes from
// `let scale = $state()` without an initializer in a TS-overlay).
//
// Expected: exactly 1 diagnostic — TS2345 at the `scale` position,
// "Argument of type 'unknown' is not assignable to parameter of type
// 'String | Number | null | undefined'".

export function $$render_broken_unknown() {
    const scale: unknown = null;

    async function __svn_tpl_check() {
        __svn_ensure_type(String, Number, scale);
    }
    void __svn_tpl_check;
}
void $$render_broken_unknown;
