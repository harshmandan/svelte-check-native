<script lang="ts">
  type Shape =
    | { kind: 'circle'; radius: number }
    | { kind: 'square'; side: number }

  let { shape }: { shape: Shape } = $props()
</script>

<!-- `{@const}` must be emitted inline as `const k = shape.kind` so the
     downstream `{#if}` narrowing works. Previously we declared
     `let k: any` at the top of the template-check body, which erased
     the discriminant type. -->
{@const k = shape.kind}
<p>kind = {k}</p>

{#if shape.kind === 'circle'}
  <p>radius: {shape.radius}</p>
{:else if shape.kind === 'square'}
  <p>side: {shape.side}</p>
{/if}

<!-- Destructuring pattern — each binding must be voided so TS6133
     doesn't fire on bindings the user only happens to reference on a
     subset of arms. -->
{@const [first, { kind: k2 }, ...rest] = [shape, shape]}
<p>{first.kind} {k2} {rest.length}</p>
