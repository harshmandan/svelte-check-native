<script lang="ts">
  import TsTemplate from './TsTemplate.svelte';
</script>
<TsTemplate on:go={(e) => e.detail.anything} on:7 on:nope />
