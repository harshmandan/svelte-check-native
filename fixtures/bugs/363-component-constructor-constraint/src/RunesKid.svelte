<script lang="ts">import type { Snippet } from 'svelte'; let { children }: { children: Snippet<[number]> } = $props();</script>
{@render children(1)}
