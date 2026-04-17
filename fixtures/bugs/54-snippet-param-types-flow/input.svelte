<script lang="ts">
    import Wrapper from './Wrapper.svelte';

    function formatId(n: number): string {
        return `#${n}`;
    }
    function joinLabels(labels: readonly string[]): string {
        return labels.join(', ');
    }
</script>

<Wrapper>
    {#snippet row({ id, label })}
        <!-- `id` is contextually typed as `number` via the Snippet<[{id, label}]>
             declaration; `label` as `string`. Without prop-satisfies weaving
             each binding reads as implicit-any and formatId/joinLabels fire
             TS7053 / TS2345 on the calls below. -->
        <p>{formatId(id)} — {label.toUpperCase()}</p>
    {/snippet}
    {#snippet header(columns)}
        <!-- `columns` is contextually typed as `readonly string[]`. The
             positional snippet binding, without weaving, ends up as `any`. -->
        <h1>{joinLabels(columns)}</h1>
    {/snippet}
</Wrapper>
