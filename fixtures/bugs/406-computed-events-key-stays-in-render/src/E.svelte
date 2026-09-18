<script lang="ts">
  const k = "go";
  interface $$Events { [k]: CustomEvent<number> }
  import { createEventDispatcher } from "svelte";
  const d = createEventDispatcher();
  d("go", 1);
</script>
