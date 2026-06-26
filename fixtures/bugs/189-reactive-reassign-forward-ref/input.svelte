<script lang="ts">
    export let value: string;
    $: value = normalize(value);
    const normalize = (v: string): string => v.trim();
</script>

<p>{value}</p>
