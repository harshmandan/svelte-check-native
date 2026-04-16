<script lang="ts">
    import { writable } from 'svelte/store';
    const parent$ = writable(0);
</script>

<p>{$parent$}</p>
