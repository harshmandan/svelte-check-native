<script lang="ts">
  let flag = true;
</script>

<!-- An in-tag `@ts-expect-error` must suppress the deliberate `dir`
     type error on the following attribute. This is the tightest lock
     on comment threading: if the comment is dropped, TS2322 surfaces;
     if the directive is threaded but the attribute does NOT error,
     TS2578 (unused '@ts-expect-error') surfaces. Zero errors proves
     both the threading and the underlying attribute check are intact. -->
<div
  // @ts-expect-error dir expects a string union, not boolean
  dir={flag}
></div>
