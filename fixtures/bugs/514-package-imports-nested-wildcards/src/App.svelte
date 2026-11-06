<script lang="ts">
  import type { Scope } from '#lib/server/Card.svelte';
  import type { Tone } from '#lib/Badge.svelte';

  const scope: Scope = 'server';
  const tone: Tone = 'info';
</script>

<p>{scope} {tone}</p>
