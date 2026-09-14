<script lang="ts">
    import { createEventDispatcher } from 'svelte';
    const dispatch = createEventDispatcher();
    dispatch('foo');
    let w = $state(0);
</script>

<p>{w}</p>
