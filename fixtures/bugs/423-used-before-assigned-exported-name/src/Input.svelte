<script lang="ts">
  export let el: HTMLElement;
  function f() { let el: HTMLElement; return el; }
  function g() { let other: HTMLElement; return other; }
  f(); g();
</script>
<div bind:this={el}></div>
