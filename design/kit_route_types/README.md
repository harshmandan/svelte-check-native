# Kit-route `$types` injection fixture

Locks the emit shape for fix #1 from the 2026-04-24 charting-lib-
bench investigation write-up (private notes).

## What

Hand-written `.ts` overlays representing what our emit SHOULD
produce for `export let data` on a `+page.svelte` Kit-route file.

Mirrors a real-world over-fire we observed on a charting-lib
bench — a `+page.svelte` that calls `topojson-client`'s `feature()`
on `data.geojson.*` — at the emit-shape level:

- `+page.ts` load returns a `Topology` with `states: GeometryCollection<...>`
- `export let data` in `+page.svelte` should be typed
  `: import('./$types.js').PageData`
- `feature(data.geojson, data.geojson.objects.states)` then picks
  the `GeometryCollection` overload → returns `FeatureCollection`
  → `.features` access resolves without TS2339.

## Validation commands

```sh
cd design/kit_route_types/fixture

# Clean overlay only (expected: 0 errors)
mv src/overlay_current_broken.ts src/overlay_current_broken.ts.hidden
tsgo --project tsconfig.json --noEmit --pretty false
# exit 0, no output

# Current broken overlay (expected: 1 error, TS2339 on features)
mv src/overlay_current_broken.ts.hidden src/overlay_current_broken.ts
tsgo --project tsconfig.json --noEmit --pretty false
# src/overlay_current_broken.ts(17,31): error TS2339: Property 'features' does not exist on type 'Feature<Point, GeoJsonProperties>'.
```

## Files

- `src/$types.d.ts` — stand-in for SvelteKit's generated types
  (PageData, LayoutData) plus topojson-client's `feature()` overload
  set (reduced to Point vs GeometryCollection).
- `src/overlay_clean.ts` — the target emit shape (`data!: PageData`).
  tsgo must accept it clean.
- `src/overlay_current_broken.ts` — the status-quo emit shape
  (`data!: any`). tsgo must fire TS2339 on `states.features` here.

## Why two overlays side-by-side

Locks both signals at once:
1. The target shape type-checks clean (fix works for legit code).
2. The current shape reproduces the bug we're fixing (fix actually
   matters — if both passed, the fix would be a no-op).

## Upstream reference

`language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts:424-440`
— the `isKitExport` branch that injects the type annotation.

`language-tools/packages/svelte2tsx/src/helpers/sveltekit.ts:26,45-54`
— `kitPageFiles` set + `isKitRouteFile(basename)` predicate.
