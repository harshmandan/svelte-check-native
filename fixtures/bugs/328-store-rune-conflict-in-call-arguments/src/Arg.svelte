<script>
  import { writable } from 'svelte/store';
  let state = writable(0);
  console.log($state);
  console.log(($state));
  new Set($state);
  $state;
</script>
