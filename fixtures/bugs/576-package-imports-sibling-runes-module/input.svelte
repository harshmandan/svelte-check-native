<script lang="ts">
    import Counter, { badge } from '#lib/Counter.svelte';
    import { CounterState, formatCount } from '#lib/Counter.svelte.js';

    const model = new CounterState();
    const label = formatCount(model.count);
</script>

<Counter {model} onStep={(payload) => void payload.next} />
{@render badge(label)}
