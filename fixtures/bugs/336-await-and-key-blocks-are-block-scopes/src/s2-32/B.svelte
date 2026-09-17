<script lang="ts">let k = $state(1);</script>
{#key k}{#snippet s()}a{/snippet}{@render s()}{/key}
{#key k}{#snippet s()}b{/snippet}{@render s()}{/key}
