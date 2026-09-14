<script lang="ts">
  let count = 0;
  let from = 'x';
  function fn(o: Record<string, unknown>) { return JSON.stringify(o); }
</script>
<p>{fn({ count: 1, from: 2 })}</p>
