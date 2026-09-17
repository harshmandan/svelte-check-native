<script>let { items } = $props(); let selected = 0;</script>
{#if items.length}{@const selected = items[0]}<b>{selected.name}</b>{/if}
<select bind:value={selected}></select>
