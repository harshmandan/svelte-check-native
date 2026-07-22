<script lang="ts">
  async function lazy() {
    return import('./Ghost.svelte')
  }
</script>
<button onclick={lazy}>go</button>
