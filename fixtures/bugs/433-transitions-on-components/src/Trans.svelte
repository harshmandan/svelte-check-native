<script lang="ts">
  import C from "./C.svelte";
  function fade(node: Element, p?: { d: number }) { return { duration: p?.d ?? 0 }; }
  function act(n: any) {}
</script>
<C transition:fade />
<C in:fade={{ d: 1 }} out:fade />
{#each [1] as i (i)}<C animate:act />{/each}
<C transition:fade></C>
