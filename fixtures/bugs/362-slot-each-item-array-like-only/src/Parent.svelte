<script lang="ts">
  import Slotter from './Slotter.svelte';
</script>
<Slotter items={new Set([1])} list={[1]} let:item>
  {item.foo}
  <svelte:fragment slot="list" let:entry>{entry.foo}</svelte:fragment>
</Slotter>
