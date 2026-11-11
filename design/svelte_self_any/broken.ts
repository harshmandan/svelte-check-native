export function render() {
    // TS2345: props must be an object.
    { __svn_create_component_any(5); }
    // TS2322: the instance is a component, not a string.
    { const inst = __svn_create_component_any({}); const s: string = inst; s; }
}
