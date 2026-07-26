// App-wide ambients a real SvelteKit project declares in src/app.d.ts.
// `App.PageData` is referenced by $types.d.ts's OutputDataShape.
declare global {
    namespace App {
        interface PageData {}
        interface Error {
            message: string;
        }
    }
}

export {};
