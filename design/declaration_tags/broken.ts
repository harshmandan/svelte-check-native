// Companion to clean.ts — deliberately-broken declaration tags.
//
// Mirrors upstream's LS fixture `declaration-tag.v5/expectedv2.json`:
//   {let label: number = $state(`${area} square pixels`)}  -> TS2322
//   {const doubled: string = area * 2}                     -> TS2322
//   {doubled === 1}                                        -> TS2367
//   {label === 'large'}                                    -> TS2367
//
// Expected diagnostics (4):
//   2x TS2322 (annotation/initialiser mismatch)
//   2x TS2367 (comparison of non-overlapping types)

declare function $state<T>(v: T): T;
declare function ensureArray<T>(a: T[]): T[];

const boxes: { width: number; height: number }[] = [
    { width: 3, height: 4 },
];

(async () => {
    for (const box of ensureArray(boxes)) {
        const area = box.width * box.height;
        void area;
        // string initialiser, number annotation -> TS2322
        let label: number = $state(`${area} square pixels`);
        void label;
        // number initialiser, string annotation -> TS2322
        const doubled: string = area * 2;
        void doubled;

        // doubled is string, compared to number -> TS2367
        (doubled === 1);
        // label is number, compared to string -> TS2367
        (label === "large");
    }
})();

export {};
