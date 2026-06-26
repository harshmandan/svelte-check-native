// Tsgo validation fixture for Svelte 5 declaration tags
// (`{const x = y}` / `{let x = y}`), upstream language-tools #3033.
//
// A declaration tag emits, inline in the current template-check scope,
// as a plain `const <decl>;` / `let <decl>;` statement — the same shape
// we already use for `{@const}`, plus a `let` variant and preserved
// type annotations (`{let label: number = ...}`).
//
// This is the "everything type-checks" case: the annotated initialisers
// match, and every binding is read. Expect ZERO diagnostics.

declare function $state<T>(v: T): T;
declare function ensureArray<T>(a: T[]): T[];

const boxes: { width: number; height: number }[] = [
    { width: 3, height: 4 },
    { width: 5, height: 7 },
];

(async () => {
    for (const box of ensureArray(boxes)) {
        // {const area = box.width * box.height}
        const area = box.width * box.height;
        void area;
        // {let label: string = $state(`${area}`)}
        let label: string = $state(`${area} square pixels`);
        void label;
        // {const doubled: number = area * 2}
        const doubled: number = area * 2;
        void doubled;

        // <p>{doubled === 1} {label === 'large'}</p>
        (doubled === 1);
        (label === "large");

        // nested <div> block with a shadowing {const area = 'nested'}
        {
            const area = "nested";
            (area);
        }

        // a `let` reassigned later is still legal
        label = "wide";
        void label;
    }
})();

export {};
