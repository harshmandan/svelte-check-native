<script lang="ts">
    let { a, b = 1 }: { a: number; b?: number; foo?: import('svelte').Snippet } = $props();
</script>

{a}{b}
