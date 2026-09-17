<script lang="ts">
  let s: string = $state('x');
  let el: HTMLDivElement | undefined = $state();
  let w: number = $state(0);
  let g: string[] = $state([]);
  let n: number = 'not a number';
</script>
<input bind:value={s as string} />
<div bind:this={el!}></div>
<div bind:clientWidth={w satisfies number}></div>
<input type="checkbox" bind:group={g as string[]} value="a" />
<svelte:window bind:innerWidth={w as number} />
<p>{s}{el}{w}{g}{n}</p>
