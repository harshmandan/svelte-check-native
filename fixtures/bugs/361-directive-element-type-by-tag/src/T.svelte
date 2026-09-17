<script lang="ts">
  function act<T extends Element>(_node: T, _cb: (n: T) => void) {}
  function tr<T extends Element>(_node: T, _cb: (n: T) => void) { return {}; }
</script>
<my-widget use:act={(n) => n.align}></my-widget>
<svelte:body use:act={(n) => n.nope} />
<svelte:window use:act={(n) => n.nope} />
<svelte:document use:act={(n) => n.nope} />
<svelte:element this={'div'} use:act={(n) => n.nope}></svelte:element>
<my-widget transition:tr={(n) => n.align}></my-widget>
<svelte:element this={'div'} transition:tr={(n) => n.nope}></svelte:element>
<div use:act={(n) => n.align} transition:tr={(n) => n.nope}></div>
