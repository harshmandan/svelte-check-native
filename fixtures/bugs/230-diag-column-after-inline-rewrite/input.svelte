<script lang="ts">
	import { createEventDispatcher } from 'svelte';
	interface $$Events { ping: CustomEvent<number>; }
	const dispatch = createEventDispatcher(); const bad: string = 123;
	dispatch('ping', 1);
	void bad;
</script>

<p>ready</p>
