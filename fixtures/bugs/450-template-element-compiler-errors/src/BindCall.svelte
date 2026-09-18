<script>
  let a = $state({ f() { return 1; } });
</script>
<input bind:value={a.f()} />
