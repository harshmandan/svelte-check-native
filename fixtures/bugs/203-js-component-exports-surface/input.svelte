<script lang="ts">
    import Child from './Child.svelte';
    const comp: Child = null as any;
    const n: number = comp.test();
    void n;
    if (comp.bar === 'foo') {
        comp.test();
    }
</script>

<Child />
