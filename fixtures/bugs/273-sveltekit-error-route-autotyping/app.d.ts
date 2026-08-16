// Stand-in for a SvelteKit app's src/app.d.ts. Kit projects declare
// the app-wide error shape on the global `App` namespace; the
// `error` prop our emit injects into +error.svelte references it as
// `App.Error`.

declare global {
    namespace App {
        interface Error {
            message: string;
        }
    }
}

export {};
