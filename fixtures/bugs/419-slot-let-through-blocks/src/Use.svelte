<script lang="ts">import List from './List.svelte'; let c = true;</script>
<List>{#if c}<li let:item>{item.nope}</li>{/if}</List>
<List>{#each [1] as _}<li let:itme>{itme}</li>{/each}</List>
<List>{#if c}<div slot="n" let:v>{v.nope}</div>{/if}</List>
<List><div><span let:item>{item}</span></div></List>
