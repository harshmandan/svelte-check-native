<script lang="ts">
    import { writable } from 'svelte/store';

    const state = writable({ count: 0 });
    // `missing` is not on the store's value type — the subscription
    // alias carries the real value type, so this fires TS2339.
    const broken = $state.missing;
    void broken;
</script>
