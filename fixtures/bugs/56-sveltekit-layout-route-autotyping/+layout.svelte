<script lang="ts">
    // +layout.svelte — our emit injects `data: LayoutData` and
    // `children?: Snippet` when the destructure omits them. Here we
    // spell `children` explicitly (common pattern) and omit `data` to
    // exercise the path where a layout takes ONLY children.
    let { children } = $props();
</script>

{@render children?.()}
