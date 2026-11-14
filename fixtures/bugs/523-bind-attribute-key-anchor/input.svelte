<script lang="ts">
	let n = 0
	let s = ''
	let title = 'x'
	let el: HTMLElement | undefined
	let tag = 'div'
	const o = { id: 'a' }
	const f = () => {}
	const act = (_: HTMLElement) => {}
	const setStr = (v: string) => { s = v }
</script>

<details hidden bind:open={n}></details>
<div hidden bind:clientWidth={null, setStr}></div>
<details class="a" bind:open={n}></details>
<div class="a" bind:clientWidth={null, setStr}></div>
<details class=a bind:open={n}></details>
<div class=a bind:clientWidth={null, setStr}></div>
<details title={s} bind:open={n}></details>
<div title={s} bind:clientWidth={null, setStr}></div>
<details title="{s}" bind:open={n}></details>
<div title="{s}" bind:clientWidth={null, setStr}></div>
<details {title} bind:open={n}></details>
<div {title} bind:clientWidth={null, setStr}></div>
<details {...o} bind:open={n}></details>
<div {...o} bind:clientWidth={null, setStr}></div>
<details class="a {s}" bind:open={n}></details>
<div class="a {s}" bind:clientWidth={null, setStr}></div>
<details onclick={f} bind:open={n}></details>
<div onclick={f} bind:clientWidth={null, setStr}></div>
<details class:x={true} bind:open={n}></details>
<div class:x={true} bind:clientWidth={null, setStr}></div>
<details style:color="red" bind:open={n}></details>
<div style:color="red" bind:clientWidth={null, setStr}></div>
<details use:act bind:open={n}></details>
<div use:act bind:clientWidth={null, setStr}></div>
<details bind:this={el} bind:open={n}></details>
<div bind:this={el} bind:clientWidth={null, setStr}></div>
<details class="a" class:x={true} bind:open={n}></details>
<div class="a" class:x={true} bind:clientWidth={null, setStr}></div>
<details class:x={true} class="a" bind:open={n}></details>
<div class:x={true} class="a" bind:clientWidth={null, setStr}></div>
<svelte:element this={tag} bind:clientWidth={null, setStr} />
<svelte:element this={tag} class="a" bind:clientWidth={null, setStr} />
{n}{title}{el}
