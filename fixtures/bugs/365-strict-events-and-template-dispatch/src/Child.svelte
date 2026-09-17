<script>
  import { createEventDispatcher } from 'svelte';
  /** @type {import('svelte').EventDispatcher<{ save: number }>} */
  const dispatch = createEventDispatcher();
</script>
<button on:click={() => dispatch('save', 1)}>x</button>
