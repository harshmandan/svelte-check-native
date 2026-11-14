<script lang="ts">
    import Child from "./Child.svelte";
    let s = 1;
</script>
<Child a={(s, 2)}>{#snippet foo()}x{/snippet}</Child>
