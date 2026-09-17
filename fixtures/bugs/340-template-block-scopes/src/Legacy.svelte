<script>export let title; export let x;</script>
{#if x}{@const title = 'a'}{title}{/if}
<h1>{x}</h1>
