<script lang="ts">
    import Child from "./Child.svelte";
    let s = 1;
    let r = {};
    let b = 1;
</script>
<Child a={(s, 2)} { ...r } />
