<script lang="ts">
    import type { Snippet } from "svelte";

    interface Props {
        children: Snippet<[{ tag: typeof tagInfo }]>;
    }

    const { children }: Props = $props();

    const tagInfo = { id: "x", count: 1 };
</script>

{@render children({ tag: tagInfo })}
