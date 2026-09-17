<script lang="ts" strictEvents>
  import { createEventDispatcher as make, createEventDispatcher as other } from 'svelte';
  const fire = make();
  const typed = other<{ nope: number }>();
  void typed;
</script>
<button on:click={() => fire('go')} on:dblclick={() => fire(7)}>x</button>
