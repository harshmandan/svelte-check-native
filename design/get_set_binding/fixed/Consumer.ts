// Target emit shape (reduction) — matches upstream's
// `__sveltets_2_get_set_binding(get, set)` call (our `__svn_` prefix).
//
// `<Child bind:value={get, set} />` emits as:
//
//     { value: __svn_get_set_binding(get, set) }
//
// TS infers `T` from `get`'s return type, then checks `set`'s parameter
// against the same `T`, then the return value `T` is assigned to
// `value`. Getter, setter, AND consumer-side `value` declaration are
// all cross-checked.
//
// Clean case: setter's parameter matches `T`. Expected tsgo: ZERO
// diagnostics.

type ChildProps = { value: string };

function consumer_site() {
    let s: string = 'hi';
    const get = () => s;
    const set = (v: string) => {
        s = v;
    };

    const _: ChildProps = {
        value: __svn_get_set_binding(get, set),
    };
    void _;
}
consumer_site;
