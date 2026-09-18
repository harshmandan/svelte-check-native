<svelte:options customElement="x-em" />
<script lang="ts">
  export let s: () => any;
</script>
{#if $$slots}<slot name="a" />{/if}
{@render s()}
