<script lang="ts">
    import Out, { Piece } from '../../515-no-such-sibling/Out.svelte';
    let n: number = Out;
</script>

<Out />
<Piece a={1} />
<p>{n}</p>
