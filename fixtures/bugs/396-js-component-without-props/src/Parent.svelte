<script lang="ts">
import Child from './Child.svelte';
import Typedef from './Typedef.svelte';
import RunesTypedef from './RunesTypedef.svelte';
import TypeComment from './TypeComment.svelte';
</script>
<Child foo={1} />
<Typedef a={1} />
<RunesTypedef a={1} />
<TypeComment b="x" />
<TypeComment b={1} />
