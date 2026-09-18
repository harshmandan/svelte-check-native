<script lang="ts">
  function helper(_a: any, _b: any) { return 1; }
  class Plain { constructor(_o: any) {} }
  const LongerName = Date;
  const ns = { Inner: Date, P: Plain };
</script>
<svelte:component this={ Date } />
<svelte:component this={helper} foo={1}>text</svelte:component>
<svelte:component this={Plain} />
<LongerName a={1} />
<ns.Inner />
<ns.P />
<LongerName>body</LongerName>
