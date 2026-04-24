// Simulated overlay of a Svelte file with three `style:` directives
// whose values are all correctly typed (string / number / null).
// Must compile with zero diagnostics.
//
// Source Svelte (conceptual):
//     <div style:color={color}
//          style:width={width}
//          style:display={show ? 'block' : 'none'}></div>

export function $$render_clean() {
    const color: string = 'red';
    const width: number = 42;
    const show: boolean = true;

    async function __svn_tpl_check() {
        __svn_ensure_type(String, Number, color);
        __svn_ensure_type(String, Number, width);
        __svn_ensure_type(String, Number, show ? 'block' : 'none');
    }
    void __svn_tpl_check;
}
void $$render_clean;
