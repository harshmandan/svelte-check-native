<script lang="ts">
  let { s }: { s: () => any } = $props();
  let v = $state(0);
</script>
<button onclick={() => v++} on:click={() => v++}></button>
<slot />
{@render s()}
