<script>
  import { writable } from 'svelte/store';
  let state = writable([1]);
  Math.max(...$state);
</script>
