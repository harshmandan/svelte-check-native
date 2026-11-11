<script lang="ts">
	import Legacy from './Legacy.svelte'
</script>

<Legacy><div><Legacy slot="badge" let:item>{item.toFixed()}</Legacy></div></Legacy>
<Legacy><svelte:fragment slot="badge"><Legacy slot="other" label="a" /></svelte:fragment></Legacy>
<Legacy><Legacy slot="badge" let:item>{item.toUpperCase()}</Legacy></Legacy>
