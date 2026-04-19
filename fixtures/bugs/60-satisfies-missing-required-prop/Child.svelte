<script lang="ts">
    let { label, count = 0 }: { label: string; count?: number } = $props()
</script>

<span>{label}: {count}</span>
