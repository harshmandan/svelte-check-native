// SvelteKit experimental remote functions live in `*.remote.ts` files.
// From svelte-check-native's perspective these are plain TypeScript
// modules — tsgo type-checks them through the regular `include` glob,
// no special handling is needed.
//
// Real-world usage wraps these in `query()` / `form()` / `command()`
// from `$app/server`, but the runtime wrappers don't affect type-
// checking of the function bodies themselves; the wrapper imports
// require a SvelteKit project structure that's out of scope for a
// minimal fixture. The point being tested here is: a `.remote.ts`
// file flows through our pipeline AND a `.svelte` file can import
// + call its exports without type errors.

export async function getUser(id: string): Promise<{ id: string; name: string }> {
    return { id, name: 'Ada' };
}

export async function getCount(): Promise<number> {
    return 7;
}
