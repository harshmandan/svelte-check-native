<script lang="ts">import Comp from './Comp.svelte';</script>
<Comp>{#snippet children()}<p>{@render children()}</p>{/snippet}</Comp>
