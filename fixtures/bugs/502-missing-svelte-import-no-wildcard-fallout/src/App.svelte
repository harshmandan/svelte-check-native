<script lang="ts">
    import Gone from './H.svelte';
    import { Piece } from './B.svelte';
    import type Kind from './J.svelte';
    import { Thing } from '$lib/Gone.svelte';
    let k: Kind | undefined = undefined;
    let num: number = Gone;
    let t: Thing = 1;
</script>

<Gone />
<Piece a={1} />
<p>{k}{num}{t}</p>
