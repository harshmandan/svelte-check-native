<script lang="ts">
  let { s }: { s: () => any } = $props();
</script>
<slot />
{@render s()}
