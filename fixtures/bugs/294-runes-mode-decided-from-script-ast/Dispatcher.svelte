<script lang="ts">
    import { createEventDispatcher } from 'svelte';
    const dispatch = createEventDispatcher();
    dispatch('foo');
</script>

<p>Call $state(0) to start</p>
