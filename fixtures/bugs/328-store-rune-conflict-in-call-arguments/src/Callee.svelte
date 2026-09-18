<script>
  import { writable } from 'svelte/store';
  let state = writable(() => 1);
  $state();
  const x = $state(1);
</script>
