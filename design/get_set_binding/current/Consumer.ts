// Current emit shape (reduction) — proves the correctness gap.
//
// A consumer writes `<Child bind:value={get, bad_set} />` where the
// setter takes a WRONG type. Our current emit at
// `crates/emit/src/lib.rs:3280-3288` (PropShape::GetSetBinding arm):
//
//     { value: (get)() }
//
// invokes the getter and assigns its return to `value`. The setter is
// never referenced. TS can't catch the mismatched setter.
//
// Expected tsgo result: ZERO diagnostics (the gap we want to close).

type ChildProps = { value: string };

// Stand-in for the child component's props literal. Models what the
// compiled consumer emit looks like today.
function consumer_site() {
    let s: string = 'hi';
    const get = () => s;
    // setter takes a number — DIFFERENT from `value: string`.
    // This is the user bug we currently miss.
    const bad_set = (n: number) => {
        void n;
    };

    const _: ChildProps = {
        // Current emit shape: invoke the getter, discard the setter.
        value: (get)(),
    };
    void _;
    // `bad_set` never referenced from emit — TS has no opportunity to
    // type-check its parameter against the value type.
    void bad_set;
}
consumer_site;
