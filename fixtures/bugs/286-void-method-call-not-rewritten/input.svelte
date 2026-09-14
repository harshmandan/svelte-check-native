<script lang="ts">
    const api = { void(a: number, b: number) { return a + b; } };
    const r: number = api.void(1, 2);
</script>

<p>{r}</p>
