// `bind:NAME={get, set}` on a DOM element, every name — one-way included —
// as svelte2tsx's attribute of the element: `"bind:NAME": get_set(get, set)`.
const setNum = (v: number) => { void v; };
const setMaybe = (v: number | undefined) => { void v; };
const setRect = (v: DOMRectReadOnly) => { void v; };
const setGroup = (v: string[]) => { void v; };
let g: string[] = [];
export function render() {
    // <div bind:clientWidth={null, setNum}>
    { svelteHTML.createElement("div", { "bind:clientWidth": (__svn_get_set_binding(null, setNum)), }); }
    // <div bind:clientWidth={null, setMaybe}>  (ours today: false TS1360)
    { svelteHTML.createElement("div", { "bind:clientWidth": (__svn_get_set_binding(null, setMaybe)), }); }
    // <div bind:contentRect={null, setRect}>
    { svelteHTML.createElement("div", { "bind:contentRect": (__svn_get_set_binding(null, setRect)), }); }
    // <img bind:naturalWidth={null, setNum}>
    { svelteHTML.createElement("img", { "alt": ``, "bind:naturalWidth": (__svn_get_set_binding(null, setNum)), }); }
    // <video bind:buffered={null, (v) => …}>
    { svelteHTML.createElement("video", { "bind:buffered": (__svn_get_set_binding(null, (v: import('svelte/elements').SvelteMediaTimeRange[]) => { void v; })), }); }
    // <input type="checkbox" bind:group={() => g, setGroup}>
    { svelteHTML.createElement("input", { "type": `checkbox`, "bind:group": (__svn_get_set_binding(() => g, setGroup)), }); }
}
