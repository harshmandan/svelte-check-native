<script lang="ts">
  import Card from "./Card.svelte";
</script>
<Card>
  {#snippet header()}<p>h</p>{/snippet}
  {#snippet children()}<p>c</p>{/snippet}
</Card>
{#snippet used()}<p>u</p>{/snippet}
{@render used()}
