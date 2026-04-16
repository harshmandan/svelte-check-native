<script>
  import { writable } from 'svelte/store';
  const count = writable(0);
  const handler1 = () => ++$count
  const handler2 = () => $count--
</script>

<button onclick={() => ++$count}>add</button>
<button onclick={() => $count--}>subtract</button>
<button onclick={() => $count++}>add</button>
