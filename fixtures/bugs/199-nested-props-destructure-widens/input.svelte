<script lang="ts">
    import AllNested from './AllNested.svelte';
    import Nested from './Nested.svelte';

    const u = { name: 'ada' };
</script>

<!-- Extra props are accepted (the widened type absorbs them), and
     omitting the nested `user` element / passing arbitrary keys to
     the all-nested component fires nothing. -->
<Nested user={u} age={1} extra={2} />
<AllNested user={{ name: 'ada', age: 1 }} whatever="ok" />
