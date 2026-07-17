<script lang="ts">
	let count = $state(0);

	const sections = [
		{
			value: 'a',
			// svelte-ignore state_referenced_locally
			open: count > 0
		}
	];

	class Persisted<T> {
		constructor(
			public key: string,
			public value: T
		) {}
	}

	const thing = new Persisted<{ title: string }>(
		// eslint-disable-next-line svelte/no-unused-svelte-ignore
		// svelte-ignore state_referenced_locally
		`key-${count}`,
		{ title: '' }
	);
</script>

<button onclick={() => count++}>{sections[0].open} / {thing.key} / {count}</button>
