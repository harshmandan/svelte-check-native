<script lang="ts">
  enum Size { Small, Large }
  class Point {
    constructor(private readonly x: number) {}
    get value(): number {
      return this.x;
    }
  }
  const p = new Point(Size.Large);
</script>

<p>{p.value}</p>
