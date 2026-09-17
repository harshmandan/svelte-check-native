<script lang="ts">
  let { a }: { a: string } = $props();
</script>
<!-- svelte-ignore slot_attribute_invalid -->
<p>{a}</p>
<!-- svelte-ignore snippet_confict -->
<p>{a}</p>
<!-- svelte-ignore a11y_missing_attribute, component_invalid_directive -->
<img src={a}>
