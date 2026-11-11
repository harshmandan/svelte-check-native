const setStr = (v: string) => { void v; };
const setNum = (v: number) => { void v; };
export function render() {
    // <div bind:clientWidth={null, setStr}>        → TS2322 on the key
    { svelteHTML.createElement("div", { "bind:clientWidth": (__svn_get_set_binding(null, setStr)), }); }
    // <div bind:contentRect={null, setStr}>        → TS2322 on the key
    { svelteHTML.createElement("div", { "bind:contentRect": (__svn_get_set_binding(null, setStr)), }); }
    // <div bind:naturalWidth={null, setNum}>       → TS2353 (not a div binding)
    { svelteHTML.createElement("div", { "bind:naturalWidth": (__svn_get_set_binding(null, setNum)), }); }
    // <div bind:offsetHeight={() => 1, setStr}>    → TS2322 on the key + TS2345 on the getter
    { svelteHTML.createElement("div", { "bind:offsetHeight": (__svn_get_set_binding(() => 1, setStr)), }); }
}
