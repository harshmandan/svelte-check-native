<script lang="ts">
import A from './A.svelte';
import B from './B.svelte';
</script>
<A a={1} on:foo={() => {}} />
<B a={1} on:foo={() => {}} />
