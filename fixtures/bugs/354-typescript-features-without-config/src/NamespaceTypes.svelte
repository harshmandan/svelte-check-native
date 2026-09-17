<script module lang="ts">
  namespace Shapes {
    export type Kind = 'a' | 'b';
    export interface Box { kind: Kind }
  }
  declare global {
    interface Window { shapeKind?: Shapes.Kind }
  }
  export const kind: Shapes.Kind = 'a';
</script>

<p>{kind}</p>
