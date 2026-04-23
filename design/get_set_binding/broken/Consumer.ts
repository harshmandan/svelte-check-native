// Target emit shape + deliberately-broken setter.
//
// Same shape as `fixed/Consumer.ts` but the setter takes the WRONG
// type. We pass `null` for the getter so the helper's type parameter
// `T` is inferred solely from the setter — making the mismatch show
// up at one predictable diagnostic site rather than cascading.
//
// Expected tsgo result: exactly ONE TS2322 on line 23 — the return
// value `T` (number, inferred from `bad_set`) is not assignable to
// `value: string`.

type ChildProps = { value: string };

function consumer_site() {
    // setter takes a number — but `value` is string. With `null` as
    // getter, `T` is inferred from the setter alone → T = number →
    // return type number → conflicts with `value: string`.
    const bad_set = (n: number) => {
        void n;
    };

    const _: ChildProps = {
        // TS2322 expected: Type 'number' is not assignable to 'string'.
        value: __svn_get_set_binding(null, bad_set),
    };
    void _;
}
consumer_site;
