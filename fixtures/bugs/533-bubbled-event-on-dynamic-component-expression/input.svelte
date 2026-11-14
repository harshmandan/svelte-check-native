<script lang="ts">
    import type { Component } from 'svelte';
    export let First: Component;
    export let Second: Component;
    export let wide = false;
</script>

<svelte:component this={wide ? First : Second} on:change />
<svelte:component this={First} on:change />
