<script lang="ts">
    let { open, onToggle }: { open: boolean; onToggle: () => void } = $props();
</script>

<button onclick={onToggle}>{open ? 'close' : 'open'}</button>
