<script>
    import Gone from "./JsGone.svelte";
</script>

<Gone />
{#await import("./JsTplGone.svelte") then m}<m.default />{/await}
