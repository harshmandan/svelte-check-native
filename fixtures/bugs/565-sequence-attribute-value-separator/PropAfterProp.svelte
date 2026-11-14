<script lang="ts">
    import Child from "./Child.svelte";
    let s = 1;
</script>
<Child b={1} a={s, 2} />
