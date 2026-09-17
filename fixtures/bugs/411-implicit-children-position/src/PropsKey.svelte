<script lang="ts">
  import Card from './Card.svelte';
  let h = () => {};
</script>
<Card>hi</Card>
<Card >hi</Card>
<Card
>hi</Card>
<Card {...{}}>hi</Card>
<Card on:x={h}>hi</Card>
<Card  on:x={h} >hi</Card>
<Card /* c */>hi</Card>
<Card title={1}>hi</Card>
<Card title={1} />
<Card
  title={1}
  other="x"
/>
