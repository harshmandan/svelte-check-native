<script lang="ts">
    import Comp from './Comp.svelte';
    let a = 1;
    let b = 's';
</script>

<Comp x={(a, b)} />
<Comp x={(b, a)} />
