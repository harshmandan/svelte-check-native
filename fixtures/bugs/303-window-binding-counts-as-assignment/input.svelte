<script lang="ts">
    let screenWidth: number;
    const LIMIT = 100;
</script>

<svelte:window bind:innerWidth={screenWidth} />
<div style="width: {(screenWidth > LIMIT ? LIMIT : screenWidth) / 2}em;"></div>
