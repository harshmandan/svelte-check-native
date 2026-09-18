<script lang="ts">import Child from './Child.svelte'; let c: any = $state();</script>
<Child bind:this={() => c, (v: string) => c = v} />
