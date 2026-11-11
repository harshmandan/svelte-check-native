<script lang="ts">
    import Child$x from './Child.svelte';
    import * as kids from './kids';
</script>

<Child$x a={1} />
<kids.$Kid a={1} />
