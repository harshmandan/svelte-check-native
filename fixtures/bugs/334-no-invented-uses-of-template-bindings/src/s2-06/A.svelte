<script lang="ts">let { onClose }: { onClose: () => void } = $props();</script>
{#if onClose}<button>x</button>{/if}
