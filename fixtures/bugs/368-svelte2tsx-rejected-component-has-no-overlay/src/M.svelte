<script lang="ts" context="module">
  function helper() {
    interface $$Events { save: CustomEvent<number> }
    return 1;
  }
</script>
<script lang="ts">
  export let a: number;
  const s: string = 1;
</script>
{a}{s}{helper()}
