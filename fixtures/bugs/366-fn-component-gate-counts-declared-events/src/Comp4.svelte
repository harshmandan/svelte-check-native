<script lang="ts">
  type Base = { save: CustomEvent<number> };
  type $$Events = Base & { close: CustomEvent<null> };
  let { a }: { a: number } = $props();
</script>
{a}
