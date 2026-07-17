<script lang="ts">
    let count: number = 0;
    const label = 'x';
</script>

<button class="btn {label}" on:click|once={() => count++} disabled>
    clicked {count} times
</button>

<style>
    .btn {
        color: red;
    }
</style>
