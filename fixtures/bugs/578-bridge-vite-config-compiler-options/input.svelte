<script lang="ts">
    import { loadCount } from './data';

    const n = await loadCount();
    const doubled = $derived(await Promise.resolve(n * 2));
</script>

<p>{doubled}</p>
<p>{await loadCount()}</p>
