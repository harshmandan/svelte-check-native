<script lang="ts">
  let s = "";           // string — clientWidth is number → TS2322
  let bad = 0;          // <div> has no naturalWidth → TS2339
</script>
<div bind:clientWidth={s}></div>
<div bind:naturalWidth={bad}></div>
