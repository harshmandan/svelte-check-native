<script>let { rows } = $props(); let id = 0; id = 1;</script>
{#each rows as { id, key = id }}{key}{/each}
