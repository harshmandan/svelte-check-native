<script lang="ts" module>
    export { badge };
</script>

<script lang="ts">
    import type { CounterState } from './Counter.svelte.js';

    let {
        model,
        onStep
    }: {
        model: CounterState;
        onStep: (payload: { next: number }) => void;
    } = $props();
</script>

<button onclick={() => onStep({ next: model.count + 1 })}>step</button>

{#snippet badge(text: string)}
    <span>{text}</span>
{/snippet}
