<script>
  import { writable } from 'svelte/store';
  let { children } = $props();
  let derived = writable(0);
</script>
{@render children($derived)}
{String($derived)}
