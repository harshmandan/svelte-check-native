<script lang="ts">
  import { writable } from 'svelte/store';
  const selected = writable<string[]>([]);
  let local: string[] = [];
</script>
<input type="radio" bind:group={$selected} value="a" />
<input type="radio" bind:group={local} value="b" />
