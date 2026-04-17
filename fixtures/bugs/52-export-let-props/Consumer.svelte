<script lang="ts">
    import Legacy from './Legacy.svelte';

    let current = $state('initial');
</script>

<!-- Consumer passes an inline callback. For the destructure on the -->
<!-- arrow parameter to be typed (not implicit-any under strict), the -->
<!-- Legacy.svelte overlay's default export must carry its synthesized -->
<!-- Props shape — `Component<{ onChange: ...; initial?: string; count?: any }>`. -->
<!-- Before Task C, the overlay typed the default as `any` whenever the -->
<!-- source used `export let` instead of `$props()`, stripping all -->
<!-- contextual type flow. -->
<Legacy onChange={(value) => (current = value.toUpperCase())} />
