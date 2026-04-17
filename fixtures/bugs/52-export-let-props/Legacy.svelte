<script lang="ts">
    // Svelte 4-style prop declarations. Our analyze extracts these
    // into a synthesized Props type on the default export so
    // consumers get proper contextual typing on destructured
    // callback parameters.
    export let onChange: (value: string) => void;
    export let initial: string = 'hello';
    export let count = 0;
</script>

<button onclick={() => onChange(initial + count)}>{initial} ({count})</button>
