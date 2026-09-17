<script>import input from './input.svelte'; let { fields } = $props();</script>
{#each fields as input}<input /> {/each}
