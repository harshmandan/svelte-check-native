<script lang="ts">
    import ChildJs from "./ChildJs.svelte";
    import ChildTs from "./ChildTs.svelte";
    import ChildNone from "./ChildNone.svelte";
    let x = $state("");
</script>

<ChildJs bind:a={x} bind:j={x} />
<ChildTs bind:a={x} bind:j={x} />
<ChildNone bind:a={x} />
