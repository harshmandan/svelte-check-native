<script lang="ts" generics="T">
  type U = $$Generic;
  export let a: T;
  export let b: U;
  const s: string = 1;
</script>
{a}{b}{s}
