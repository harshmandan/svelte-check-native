<script lang="ts">
  let x = 0;
  // Params optional: a bare `transition:fade` emits a one-argument
  // call, so a required second parameter fires a genuine TS2554 that
  // upstream also reports — this fixture locks in-tag comment
  // parsing, not transition arity.
  function fade(_n: Node, _o?: object) { return {}; }
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
