// Broken companion: `bar` is boolean, comparing against a string must
// fire exactly one TS2367 ("This comparison appears to be
// unintentional because the types 'boolean' and 'string' have no
// overlap") on line 10, col 5 — the upstream `getters` LS fixture's
// expected diagnostic. Pre-change the exports surface fell to `{}` and
// `comp.bar` widened to `any`, so nothing fired.
import ComponentWithGetters from './component.svelte.svn.js';

const comp: ComponentWithGetters = null as any;
if (comp.bar === 'foo') {
    comp.test();
}
