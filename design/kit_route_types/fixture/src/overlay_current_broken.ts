// Current-behaviour emit: `let data!: any`. With `any` flowing
// into `feature(data.geojson, data.geojson.objects.states)`, TS
// picks the first `feature()` overload (Point → Feature<Point>).
// `.features` does NOT exist on that return → TS2339.
//
// Copy the user code VERBATIM (no escape-hatch casts). If tsgo
// fires TS2339 here, the fixture has reproduced the
// charting-lib-bench cluster-A root cause at its source.

import { feature, type PageData } from './$types.js';
declare const _pinPageData: PageData;
void _pinPageData;

async function $$render_current() {
    let data!: any;
    const states = feature(data.geojson, data.geojson.objects.states);
    const _nFeatures = states.features.length;
    //                        ^ expected TS2339 — features doesn't exist on Feature<Point>
    void _nFeatures;
}
void $$render_current;
