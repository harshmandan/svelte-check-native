# Ignore-region diagnostic filter — fixture

Locks root cause #3 from the 2026-04-24 investigation write-up in private notes:
the missing diagnostic filter that drops errors inside
emit-synthesised scaffolding regions.

## The problem

tsgo genuinely fires type-strict errors on user code that passes
through emit-synthesised scaffolding (e.g. component
`$$bindings` trails, intermediate helper calls). These errors
are false positives from the user's perspective — the reported
position is in code the user didn't write.

Upstream svelte2tsx marks these regions with
`/*Ωignore_startΩ*/…/*Ωignore_endΩ*/` comment pairs, and
upstream svelte-check's mapper
(`language-server/src/plugins/typescript/features/
DiagnosticsProvider.ts::mapAndFilterDiagnostics`) drops any
diagnostic whose start offset is inside such a region.

Upstream's filter:
```ts
// features/utils.ts:102-109
export function isInGeneratedCode(text: string, start: number, end: number = start) {
    const lastStart = text.lastIndexOf(IGNORE_START_COMMENT, start);
    const lastEnd = text.lastIndexOf(IGNORE_END_COMMENT, start);
    const nextEnd = text.indexOf(IGNORE_END_COMMENT, end);
    return (lastStart > lastEnd || lastEnd === nextEnd) && lastStart < nextEnd;
}
```

## This fixture

`src/overlay_with_ignore_regions.ts` contains a synthesised
overlay with TWO identical TS2322 errors:
- Line 20, inside `/*svn:ignore_start*/…/*svn:ignore_end*/`
  markers. This simulates scaffolding — must be dropped by
  the filter.
- Line 26, no markers. This is user code — must surface.

We use `/*svn:ignore_start*/` / `/*svn:ignore_end*/` as our
marker pair (not upstream's Ω pair) because (a) the marker is
implementation-private and can carry our prefix for clarity,
(b) Ω is UTF-8 multibyte which complicates byte-offset math on
the tsgo side.

## Validation

```sh
cd design/ignore_region_filter/fixture
tsgo --project tsconfig.json --noEmit --pretty false
# expected output (pre-filter):
# src/overlay_with_ignore_regions.ts(20,11): error TS2322: ...
# src/overlay_with_ignore_regions.ts(26,11): error TS2322: ...
```

After implementing the filter in our mapper, the line-20 error
must be dropped and the line-26 error must still surface.

## Upstream references

- `language-tools/packages/language-server/src/plugins/typescript/features/utils.ts:86-109`
  — `IGNORE_START_COMMENT`, `IGNORE_END_COMMENT`, `isInGeneratedCode`.
- `language-tools/packages/language-server/src/plugins/typescript/features/DiagnosticsProvider.ts:148-229`
  — `mapAndFilterDiagnostics`, the filter pipeline.
- `language-tools/packages/svelte2tsx/src/utils/ignore.ts`
  — `surroundWithIgnoreComments` helper used by svelte2tsx emit.
