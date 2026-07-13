// Tsgo validation fixture for zero-config `+error.svelte` props
// (upstream language-tools d6536401, svelte-check 4.7.2).
//
// Upstream's ExportedNames.ts now types a destructured `$props()`
// member named `error` as `App.Error` when the file's basename is
// `+error`. Our emit synthesizes the same shape into the overlay's
// $$ComponentProps. This fixture proves the shape mechanics:
//
//   - `App.Error` referenced inside a render-fn-scoped type alias
//     resolves against the global `App` namespace (declared by the
//     user's app.d.ts / @sveltejs/kit ambients in a real project)
//   - the destructured `error` flows the ambient type contextually —
//     `error.message` reads clean.
//
// This file is the clean case: zero diagnostics expected.

declare function $props<P>(): P;

declare global {
    namespace App {
        interface Error {
            message: string;
        }
    }
}

async function $$render_fixture() {
    type $$ComponentProps = { error: App.Error; };

    let { error }: $$ComponentProps = $props();

    console.log(error.message);
    return { props: undefined as any as $$ComponentProps };
}
$$render_fixture;
export {};
