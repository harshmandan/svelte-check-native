<script lang="ts">
    // @ts-nocheck
    import Gone from "./NoCheckGone.svelte";
</script>

<Gone />
{#await import("./NoCheckTplGone.svelte") then m}<m.default />{/await}
