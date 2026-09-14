<script lang="ts">
  let count = 0;
  let s = 'a';
  function fn(o: Record<string, unknown>) { return JSON.stringify(o); }
</script>
<p>{fn({ $count: 1 })} {s.replace(/\$count/, '')}</p>
