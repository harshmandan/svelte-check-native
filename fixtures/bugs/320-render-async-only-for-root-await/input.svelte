<script lang="ts">
  let data: unknown;
  { data = await fetch('/api'); }
  if (data) { const more = await fetch('/more'); console.log(more); }
</script>
<p>{data}</p>
