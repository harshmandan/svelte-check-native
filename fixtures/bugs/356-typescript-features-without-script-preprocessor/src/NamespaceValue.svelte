<script module lang="ts">
  namespace Shapes {
    export namespace Inner {
      export type Kind = 'a';
    }
    export const count = 1;
  }
  export const total = Shapes.count;
</script>

<p>{total}</p>
