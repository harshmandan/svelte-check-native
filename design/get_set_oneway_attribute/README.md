# `get_set_oneway_attribute/`

Locks the shape of a get/set binding on a DOM element, `bind:NAME={get, set}`.
svelte2tsx passes every such binding other than `bind:this` to the element as
an attribute, `"bind:NAME": __sveltets_2_get_set_binding(get, set)`, one-way
names (`clientWidth`, `contentRect`, `naturalWidth`, `duration`, `buffered`,
...) included (Binding.ts: a get/set binding never enters the one-way
branches). The element's attribute type from `svelte/elements` is what checks
the setter, so a setter that accepts a wider type (`number | undefined` for
`clientWidth`) is fine, and a binding the element does not have is an
unknown attribute.

`svelte` has to resolve:

```sh
ln -s /path/to/a/workspace/node_modules design/get_set_oneway_attribute/node_modules
tsgo -p design/get_set_oneway_attribute/tsconfig.json
```

Expected: `clean.ts` silent; `broken.ts` firing exactly

- `(5,41)` TS2322 — `string` setter on `bind:clientWidth`
- `(7,41)` TS2322 — `string` setter on `bind:contentRect`
- `(9,41)` TS2353 — `bind:naturalWidth` on a `<div>`
- `(11,41)` TS2322 and `(11,85)` TS2345 — getter and setter disagree on `bind:offsetHeight`
