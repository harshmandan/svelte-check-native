// @ts-nocheck
// Byte-shape of `svelte-kit sync`'s generated proxy module (written next
// to $types.d.ts when the user's +page.server.ts imports from './$types'):
// the user's explicit `: PageServerLoad` / `: Actions` annotations are
// stripped so `typeof import('./proxy+page.server.js').load` infers the
// LITERAL return shape, and re-asserted as trailing statements.
import type { Actions, PageServerLoad } from './$types';

export const load = async () => {
    return { title: 'Hello', count: 3 };
};

export const actions = {
    default: async () => {
        return { ok: true };
    }
};
;null as any as PageServerLoad;;null as any as Actions;
