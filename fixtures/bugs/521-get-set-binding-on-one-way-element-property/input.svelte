<script lang="ts">
	let width = 0
	const setNum = (v: number) => { width = v }
	const setMaybe = (v: number | undefined) => { width = v ?? 0 }
	const setStr = (v: string) => { width = v.length }
	const setRect = (v: DOMRectReadOnly) => { void v }
</script>

<div bind:clientWidth={null, setNum}></div>
<div bind:clientWidth={null, setMaybe}></div>
<div bind:clientWidth={null, setStr}></div>
<div bind:contentRect={null, setRect}></div>
<div bind:contentRect={null, setStr}></div>
<img alt="" bind:naturalWidth={null, setNum} />
<video bind:duration={null, setStr}></video>
<div bind:offsetHeight={() => width, setStr}></div>
<div contenteditable bind:clientHeight={null, setStr}></div>
