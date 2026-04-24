// Broken: text+mustache form `style:height="{container_height * scaleRatio}px"`.
// Emits as `__svn_ensure_type(String, Number, \`${container_height * scaleRatio}px\`)`.
// The `*` operator inside the template literal fires TS18046 against
// `container_height` (typed `unknown`). The outer ensure_type call
// doesn't itself fire — template literals implicitly coerce via
// toString, which TS accepts for unknown without complaint. The
// inner arithmetic is the diagnostic source, matching upstream's
// behavior on IFrame.svelte L139:68.
//
// Expected: exactly 1 diagnostic — TS18046 at `container_height`.

export function $$render_broken_textmustache() {
    const container_height: unknown = null;
    const scaleRatio: number = 2;

    async function __svn_tpl_check() {
        __svn_ensure_type(String, Number, `${container_height * scaleRatio}px`);
    }
    void __svn_tpl_check;
}
void $$render_broken_textmustache;
