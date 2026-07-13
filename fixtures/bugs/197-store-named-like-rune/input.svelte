<script lang="ts">
    import { writable } from 'svelte/store';

    // A Svelte-4 store literally named `state`: `$state.count` is a
    // subscription to it, not rune usage. The subscription alias must
    // be declared so the member access resolves against the store's
    // value type.
    const state = writable({ count: 0 });
    const doubled = $state.count * 2;
    void doubled;
</script>

<p>{$state.count}</p>
