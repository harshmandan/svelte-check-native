<script lang="ts">
  let x = 0;
  function fade(_n: Node, _o: object) { return {}; }
</script>

<!-- In-tag JS comments (Svelte 5) must not break parsing. Pre-fix the
     parser treated `//` as the start of `/>` and emitted a fatal
     malformed-open-tag error, skipping the whole file. -->
<div
  // a line comment
  class="foo"
  /* a block comment */
  data-x={x}
  transition:fade
></div>
