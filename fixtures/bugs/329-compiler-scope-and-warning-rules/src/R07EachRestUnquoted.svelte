<script>let { rows } = $props();</script>
{#each rows as [a, ...rest]}<input bind:value={rest[0]}>{a}{/each}
