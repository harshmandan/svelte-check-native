<script lang="ts">
  let { data } = $props();
  const needsNumber = (n: number) => n;
</script>
<p>{needsNumber(data.title)}</p>
