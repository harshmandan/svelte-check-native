<script lang="ts">
    import { fade } from 'svelte/transition';
</script>

<div transition:fade={{ duration: 'not-a-number' }}>x</div>
<div animate:undefinedAnim>y</div>
