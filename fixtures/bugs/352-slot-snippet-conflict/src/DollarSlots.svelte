<script lang="ts">
  export let s: () => any;
  const has = !!$$slots;
</script>
{#if has}{@render s()}{/if}
