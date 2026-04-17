<script lang="ts">
    import CircleView from './CircleView.svelte';
    import SquareView from './SquareView.svelte';

    type Circle = { kind: 'circle'; radius: number };
    type Square = { kind: 'square'; size: number };
    type Shape = Circle | Square;

    let { shape }: { shape: Shape } = $props();
</script>

<!-- `shape.radius` / `shape.size` are only on their respective branches. -->
<!-- The {#if} / {:else if} / {:else} narrowing must survive into the -->
<!-- component-prop check so tsgo narrows `shape` in each arm. -->
{#if shape.kind === 'circle'}
    <CircleView radius={shape.radius} />
{:else if shape.kind === 'square'}
    <SquareView size={shape.size} />
{:else}
    <span>impossible</span>
{/if}
