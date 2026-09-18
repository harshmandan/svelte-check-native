<script lang="ts">
import Child from './Child.svelte';
import Child2 from './Child2.svelte';
import Child3 from './Child3.svelte';
let child: Child;
let child2: Child2;
let child3: Child3;
function run() {
	const n: number = child.validate();
	const m: number = child2.check();
	const o: boolean = child3.first;
	return [n, m, o];
}
</script>

<Child bind:this={child} label="a" />
<Child2 bind:this={child2} label="a" />
<Child3 bind:this={child3} label="a" />
<button on:click={run}>run</button>
