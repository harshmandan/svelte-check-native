<script>
  const a = 1;
  let b = 2;
  [b = 1, ...a] = [1, 2];
  ({ b, c: a = 3 } = {});
</script>
