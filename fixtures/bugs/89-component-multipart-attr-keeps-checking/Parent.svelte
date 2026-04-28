<svelte:options runes />
<script lang="ts">
    import Child from './Child.svelte'

    let cls = 'extra'
    let count: number = 5
    // Wrong type: number passed where string is required.
    let bogus: number = 42
</script>

<!--
  Reviewer item #5 (medium). Pre-fix, a multi-part interpolated
  quoted attr like `class="a {cls} c"` disqualified the WHOLE
  component instantiation in our analyzer — every other prop /
  event / binding on the component silently went un-checked. The
  fix replaces the disqualify-all behavior with `__svn_any()` for
  the interpolated prop alone, so the rest of the component still
  type-checks. Here `label={bogus}` (number → string mismatch)
  must fire TS2322 even though `class` is interpolated.
-->
<Child class="a {cls} c" label={bogus} {count} />
