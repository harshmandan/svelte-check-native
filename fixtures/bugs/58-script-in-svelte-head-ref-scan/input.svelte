<script lang="ts">
  // `useThing` is imported for the sole purpose of being called from an
  // attribute on a nested `<script>` element inside `<svelte:head>`. The
  // top-level section parser sees two `<script>` tags and classifies the
  // second as a duplicate, but we still need to scan its opening-tag
  // attributes for identifier references so `useThing` doesn't read as
  // TS6133 "declared but never read".
  import { useThing } from './use-thing'
</script>

<svelte:head>
  <script
    src="https://example.com/script.js"
    async
    defer
    onload={useThing('hi')}></script>
</svelte:head>
