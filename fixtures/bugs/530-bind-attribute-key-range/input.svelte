<script lang="ts">
	let n = 0
	let s = ''
	const setStr = (v: string) => { s = v }
</script>

<details bind:open={n}></details>
<details bind:open={ n }></details>
<details bind:open="{n}"></details>
<details bind:open={n} title="t"></details>
<details bind:open={n}  ></details>
<details bind:open={n} class:x={true}></details>
<details bind:open={n} hidden></details>
<details bind:open={n} />
<details  bind:open={n}></details>
<details
	bind:open={n}
	title="t"
></details>
<div bind:clientWidth={null, setStr}></div>
<div bind:clientWidth={null,setStr}></div>
<div bind:clientWidth={null, setStr} title="t"></div>
<details title="t" bind:open={n}></details>
<details class:x={true} bind:open={n}></details>
{n}{s}
