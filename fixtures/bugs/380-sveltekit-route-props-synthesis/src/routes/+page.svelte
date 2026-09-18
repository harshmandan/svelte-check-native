<script lang="ts">
  // Only the props SvelteKit passes are typed on a route file, so a
  // destructured `heading` is not a property of the props type.
  let { data, heading } = $props();
</script>
{data.title}{heading}
