<script lang="ts">
  import RunesCard from './RunesCard.svelte';
  import RunesKid from './RunesKid.svelte';
  import LegacyCard from './LegacyCard.svelte';
  import Kid from './Kid.svelte';
  import { X } from './lib';
</script>
<RunesCard><div slot="header"></div></RunesCard>
<RunesKid let:x>{x}</RunesKid>
<LegacyCard><div slot="footre">x</div></LegacyCard>
<LegacyCard><div slot="footer" let:a>{a.toFixed()}</div></LegacyCard>
<Kid let:x let:y={why}>{x}{why}</Kid>
<Kid let:x={{ a }}>{a}</Kid>
<X on:save={(e) => e.detail.foo()} />
