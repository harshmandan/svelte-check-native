<script lang="ts">
    // Wrong target types. Must each fire TS2322 at the bind value
    // expression position.
    const state: { checkedWrong?: string; filesWrong?: number } = {}
</script>

<input type="checkbox" bind:checked={state.checkedWrong} />
<input type="file" bind:files={state.filesWrong} />
