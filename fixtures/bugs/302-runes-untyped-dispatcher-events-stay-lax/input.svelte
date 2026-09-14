<script lang="ts">
    import Child from './Child.svelte';
    const c = new Child({ target: document.body });
    c.$on('bar', () => {});
</script>
