<script module>
import {writable} from 'svelte/store';
export const state = writable(0);
</script>
<script>
export let a;
$state;
</script>
<button on:click={() => a}>x</button>
