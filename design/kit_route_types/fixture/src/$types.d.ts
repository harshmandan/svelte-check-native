// Mirrors what SvelteKit generates in .svelte-kit/types/**/$types.d.ts
// Simplified to the single shape the Choropleth fixture exercises:
// the load() returns a `Topology<{states: GeometryCollection<P>}>`
// whose `objects.states` IS a GeometryCollection (not a single Point),
// which in turn selects topojson-client's FeatureCollection overload.

// Minimal GeoJSON + topojson-specification stand-ins.
export interface GeoJsonProperties {
    [name: string]: unknown;
}
export interface Point {
    type: 'Point';
    coordinates: [number, number];
}
export interface Feature<G = unknown, P = GeoJsonProperties> {
    type: 'Feature';
    geometry: G;
    properties: P;
    id?: string | number;
}
export interface FeatureCollection<G = unknown, P = GeoJsonProperties> {
    type: 'FeatureCollection';
    features: Array<Feature<G, P>>;
}
export interface GeometryCollection<P = GeoJsonProperties> {
    type: 'GeometryCollection';
    geometries: unknown[];
    _brand_GC: P;
}
export interface Topology<O = unknown> {
    type: 'Topology';
    objects: O;
    _brand_T: unknown;
}

// topojson-client `feature` overload set, reduced to the two shapes
// that matter for the parity case: Point vs GeometryCollection.
export function feature<P extends GeoJsonProperties = GeoJsonProperties>(
    topology: Topology,
    object: Point,
): Feature<Point, P>;
export function feature<P extends GeoJsonProperties = GeoJsonProperties>(
    topology: Topology,
    object: GeometryCollection<P>,
): FeatureCollection<Point, P>;
export function feature(topology: Topology, object: unknown): unknown;

// The Kit-generated PageData shape. This is what our emit MUST
// reference via `import('./$types.js').PageData` for typed
// consumers of `export let data`.
export interface PageData {
    geojson: Topology<{
        states: GeometryCollection<{ name: string }>;
        counties: GeometryCollection<{ name: string }>;
    }>;
    population: Array<{
        state: string;
        county: string;
        DP05_0001E: string;
    }>;
}

// Layout variant, for fixture completeness.
export interface LayoutData {
    theme: 'light' | 'dark';
}
