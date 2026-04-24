// Expected shape of our emit for a `+page.svelte` that contains:
//
//   <script lang="ts">
//     import { feature } from './$types.js'; // stand-in for topojson-client
//     export let data;
//     const states = feature(data.geojson, data.geojson.objects.states);
//     const nFeatures = states.features.length; // must resolve — features exists on FeatureCollection
//   </script>
//
// The LOAD-BEARING line is `let data!: import('./$types.js').PageData;`.
// With that annotation, `data.geojson.objects.states` is a
// GeometryCollection, which routes `feature(...)` to the overload
// returning FeatureCollection. `.features` resolves.
//
// Without the annotation (current behaviour: `let data!: any;`), TS
// picks the `Point` overload → returns Feature<Point> → `.features`
// TS2339 — the 58-count cluster-A blast on a charting-lib
// bench's Choropleth map route.

import { feature, type PageData } from './$types.js';

async function $$render_clean() {
    let data!: PageData;
    const states = feature(data.geojson, data.geojson.objects.states);
    const _check: number = states.features.length;
    void _check;

    const _propertyAccess: string | undefined = states.features[0]?.properties.name as any;
    void _propertyAccess;

    const population = data.population.map((d) => ({
        id: d.state + d.county,
        pop: +d.DP05_0001E,
    }));
    void population;
}
void $$render_clean;
