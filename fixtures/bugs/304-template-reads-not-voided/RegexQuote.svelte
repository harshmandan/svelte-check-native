<script lang="ts">
  import { writable } from 'svelte/store';
  let count = writable(0);
  let s = 'a';
  let tail = 'b';
</script>
<p>{s.replace(/"/g, '') + tail + $count}</p>
