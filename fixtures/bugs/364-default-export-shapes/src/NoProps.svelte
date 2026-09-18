<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  const d = createEventDispatcher<{ x: number }>();
</script>
<button on:click={() => d('x', 1)}>x</button>
