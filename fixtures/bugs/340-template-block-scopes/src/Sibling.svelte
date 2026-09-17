<script>let { items } = $props(); let v = 0;</script>
{#if items}{@const v = 1}{v}{:else}{@const v = 2}{v}{/if}
<input bind:value={v}>
