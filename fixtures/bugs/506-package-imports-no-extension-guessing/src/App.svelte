<script lang="ts">
  import Card, { type Scope } from '#lib/Card.svelte';
  import { twice } from '#lib/util.js';
  import { twice as guessed } from '#lib/util';

  const ok: Scope = 'org';
</script>

<Card scope={ok} />
<p>{twice(2)} {guessed(2)}</p>
