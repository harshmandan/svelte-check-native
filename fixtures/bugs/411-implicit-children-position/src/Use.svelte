<script lang="ts">
  import Named from './Named.svelte';
  let h = () => {};
</script>
<Named>text</Named>
<Named >text</Named>
<Named title="t">text</Named>
<Named title="t"  >text</Named>
<Named title={'t'}>text</Named>
<Named {...{title: 't'}}>text</Named>
<Named title="t" on:click={h}>text</Named>
<Named on:click={h} title="t">text</Named>
<Named title="t" a="b">text</Named>
<Named
  title="t"
>text</Named>
<Named title
>text</Named>
<Named title="t" /* c */>text</Named>
