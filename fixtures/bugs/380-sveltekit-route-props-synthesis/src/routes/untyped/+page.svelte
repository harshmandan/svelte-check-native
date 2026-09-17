<script lang="ts">
  // Nothing SvelteKit-typed in the destructure: the props stay `any`.
  let { heading = '' } = $props();
</script>
{heading}
