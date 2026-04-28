<script lang="ts">
// `createEventDispatcher` imported but never CALLED — leaving the
// default-export shape selection to honour the AST. Substring
// detection used to flip on the import token and force the iso
// shape, surfacing the same Parameters<typeof Comp> fallout as
// the comment-mention case (#85).
import { createEventDispatcher } from 'svelte'
void createEventDispatcher

let { value }: { value: number } = $props()
</script>

<span>{value}</span>
