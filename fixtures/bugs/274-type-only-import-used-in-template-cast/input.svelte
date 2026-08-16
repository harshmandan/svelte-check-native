<script lang="ts">
  import { getDuration, type AppVideo } from './helpers'
  let { item }: { item: unknown } = $props()
</script>

<div>{getDuration(item as AppVideo)}</div>
