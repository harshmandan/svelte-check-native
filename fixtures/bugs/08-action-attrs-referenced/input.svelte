<script lang="ts">
    import type { Action } from 'svelte/action';

    const myAction: Action<HTMLElement, undefined, { 'on:custom': (e: CustomEvent) => void }> = (node) => {
        return {};
    };
</script>

<div use:myAction on:custom={(e) => console.log(e.detail)}></div>
