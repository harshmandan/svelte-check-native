<script lang="ts">
    const greeting: string = 'hi';
</script>

<p>{greeting}</p>
{#snippet row(b)}<p>{b}</p>{/snippet}
{#snippet pair(a = 1 < 2, b)}<p>{a}{b}</p>{/snippet}
{@render row('x')}
{@render pair(true, 1)}
