<script lang="ts">
  // value→slot direction (l003): the bound value must be assignable to
  // the slot. A widened union has an arm the slot can't accept → TS2322
  // (the old slot→value direction silently accepted this). A nullable
  // target is fine — the slot itself is `boolean | null | undefined`.
  let widened: boolean | string = false;
  let nullableOk: boolean | null = null;
</script>
<input type="checkbox" bind:checked={widened} />
<input type="checkbox" bind:checked={nullableOk} />
