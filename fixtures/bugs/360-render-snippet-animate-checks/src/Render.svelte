<script lang="ts">
  import Comp from './Comp.svelte';
  let { notASnippet }: { notASnippet: () => string } = $props();
  function plain() {}
  function tooFew(_node: Element) { return 5; }
  function thunk(_node: Element, _a: any, _p: any) { return () => {}; }
  function good(_node: Element, _a: any) { return { duration: 1 }; }
  let list = $state([1]);
</script>
{#snippet s()}<p>x</p>{/snippet}
{@render plain()}
{@render notASnippet()}
{@render s()}
<Comp onclick={s} />
{#each list as i (i)}<div animate:tooFew>{i}</div>{/each}
{#each list as i (i)}<div animate:thunk>{i}</div>{/each}
{#each list as i (i)}<div animate:good>{i}</div>{/each}
