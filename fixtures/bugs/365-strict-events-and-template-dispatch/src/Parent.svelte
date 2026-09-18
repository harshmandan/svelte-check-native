<script lang="ts">
  import A from './A.svelte';
  import B from './B.svelte';
  import Strict from './Strict.svelte';
  import Child from './Child.svelte';
  import Styled from './Styled.svelte';
  import Head from './Head.svelte';
  import LaxCase from './LaxCase.svelte';
  import JsBubble from './JsBubble.svelte';
</script>
<A on:foo={() => {}} />
<B on:foo={() => {}} />
<Strict on:sav />
<Strict on:save={(e) => e.detail.anything} />
<Child on:save={(e) => e.detail.toUpperCase()} />
<Styled on:foo={() => {}} />
<Head on:foo={() => {}} />
<LaxCase on:foo={() => {}} />
<JsBubble on:click={(e) => e.foo} />
