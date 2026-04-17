// Phase-A-only svelte module shim. The real emit pipeline uses
// crates/typecheck/src/svelte_shims_fallback.d.ts — this fixture
// reproduces just enough of it to let the fixture type-check when
// the real svelte package isn't on disk.

declare module 'svelte' {
    export type Snippet<Parameters extends any[] = []> = {
        (...args: Parameters): any;
    };

    export type Component<
        Props extends Record<string, any> = Record<string, any>,
    > = (...args: any[]) => { props: Props };
}

declare module 'svelte/store' {
    export interface Readable<T> {
        subscribe(run: (v: T) => void, invalidate?: () => void): () => void;
    }
    export interface Writable<T> extends Readable<T> {
        set(value: T): void;
        update(fn: (v: T) => T): void;
    }
    export function writable<T>(value?: T): Writable<T>;
    export function readable<T>(value?: T): Readable<T>;
    export function get<T>(store: Readable<T>): T;
}
