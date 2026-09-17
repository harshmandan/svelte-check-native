<script lang="ts">
  let { s }: { s: () => any } = $props();
</script>
<div>{@render s()}</div>
{#if true}<p><slot name="z" /></p>{/if}
