// svelte's own wildcard: every unresolved `.svelte` import lands here.
declare module '*.svelte' {
	const component: any;
	export default component;
}
