<script lang="ts">
  import C from './C.svelte';
  import E from './E.svelte';
  let ref: E;
  function f() { ref.$on('save', (e) => e.detail); }
</script>
<C a={1}>text</C>
<E on:save={(e) => e.detail} />
{f()}
