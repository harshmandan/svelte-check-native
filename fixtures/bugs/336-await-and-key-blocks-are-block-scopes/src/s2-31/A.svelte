<script lang="ts">let p = Promise.resolve(1); let q = Promise.resolve(2);</script>
{#await p}{@const x = 1}{x}{:then v}{v}{/await}
{#await q}{@const x = 2}{x}{:then w}{w}{/await}
