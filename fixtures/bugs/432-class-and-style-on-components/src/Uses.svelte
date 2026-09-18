<script lang="ts">
  import C from "./C.svelte";
  let x = true;
  let y = true;
  let s = "red";
</script>
<C class:a={x} />
<C class:b />
<C {...{}} class:y />
<C style:color={s} />
<div class:nope></div>
