<script lang="ts">
  enum Size { Small }
  function logged(value: unknown, _context: unknown): any {
    return value;
  }
  @logged
  class Point {}
  const p = new Point();
</script>

<p>{String(p)}{Size.Small}</p>
