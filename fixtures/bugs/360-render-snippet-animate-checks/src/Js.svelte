<script>
  import Comp from './Comp.svelte';
  function plain() {}
</script>
{#snippet s()}<p>x</p>{/snippet}
{#snippet t(/** @type {number} */ n)}<p>{n}</p>{/snippet}
{@render s()}
{@render t(1)}
{@render plain()}
<Comp onclick={s} />
