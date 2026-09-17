<script lang="ts">
  import type { Snippet } from 'svelte';
  import Comp from './Comp.svelte';
  let { children }: { children: Snippet } = $props();
</script>
<Comp children={children}>text</Comp>
<Comp>{#snippet other()}<p>o</p>{/snippet}{@render other()}{@render children()}</Comp>
{@render children()}
