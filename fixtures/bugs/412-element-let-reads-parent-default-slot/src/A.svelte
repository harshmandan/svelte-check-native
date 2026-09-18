<script lang="ts">import List from './List.svelte';</script>
<List><li let:itme>{itme}</li></List>
<List><li let:item>{item.toFixed()}</li></List>
<List><svelte:fragment let:item={it}>{it.nope}</svelte:fragment></List>
