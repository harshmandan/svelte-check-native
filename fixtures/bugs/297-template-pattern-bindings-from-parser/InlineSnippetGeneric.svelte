<script lang="ts">
    import Child from './Child.svelte';
</script>

<Child>
    {#snippet row(a: Map<() => void, string>)}{a.size}{/snippet}
</Child>
