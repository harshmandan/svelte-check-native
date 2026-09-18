<script lang="ts">
  export let s: () => any;
</script>
{@render s()}
<slot name="a" />
<slot />
<slot name="a" />
