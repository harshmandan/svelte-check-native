<script lang="ts">
  export let prop: string;
  let button: HTMLButtonElement;
  let count = 0;
  $: button, prop, count, console.log("button:", button);
</script>
<button onclick={() => count++} bind:this={button}>{prop}</button>
