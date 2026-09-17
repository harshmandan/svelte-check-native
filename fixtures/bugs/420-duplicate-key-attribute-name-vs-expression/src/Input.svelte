<script lang="ts">
  export let fn: () => void;
  export let o: { class?: string };
  let n = 1;
</script>
<svelte:window on:keydown={fn} on:keydown />
<button on:click={fn} on:click>x</button>
<div {...o} class="c"></div>
{#if n}
  <span role="none" on:click on:click={fn}></span>
{/if}
<div>{({ "a": 1, "a": 2 }).a}</div>
<p title={({ "b": 1, "b": n }).b}></p>
<svelte:element this="div" role="none" on:click={fn} on:click></svelte:element>
