<script lang="ts">
  import Card, { type Scope } from '#lib/Card.svelte';
  import { version } from '#lib';

  const ok: Scope = 'org';
  const bad: Scope = 'nope';
</script>

<Card scope={ok} />
<p>{version} {bad}</p>
