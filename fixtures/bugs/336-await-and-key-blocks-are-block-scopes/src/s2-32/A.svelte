<script lang="ts">let k = $state(1);</script>
{#key k}{@const x = 1}{x}{/key}
{#key k}{@const x = 2}{x}{/key}
