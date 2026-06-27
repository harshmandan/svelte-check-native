<script lang="ts">
  let contentEditable = true;
  let v = 0;
</script>

<!-- HTML element: camelCase names lowercase to match svelte-jsx typings -->
<div className="x" tabIndex={1} {contentEditable}></div>

<!-- SVG element: case-sensitive SVG attr names are preserved -->
<svg viewBox="0 0 10 10" preserveAspectRatio="xMidYMid"></svg>

<!-- Custom element (autonomous + customized built-in): case preserved -->
<my-widget dataValue="x"></my-widget>
<div is="my-thing" dataValue={v}></div>
