<script lang="ts">
    async function getNumber(): Promise<number> {
        return 42;
    }
</script>

<p>{await getNumber()}</p>
