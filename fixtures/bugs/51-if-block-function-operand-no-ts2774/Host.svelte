<script lang="ts">
    import type { Component } from 'svelte';
    import Ghost from './Ghost.svelte';

    // A Svelte-5 context object exposing a set of non-nullable component
    // functions. `{#if editable && ctx.GhostButton && …}` is a common
    // template shape — polling the context for a bindable component
    // reference before instantiating it.
    type Ctx = {
        GhostButton: Component<{ label: string }>;
    };

    let {
        ctx,
        editable,
        imgFn,
        items,
        maxItems = 5,
    }: {
        ctx: Ctx;
        editable: boolean;
        imgFn: (src: string) => string;
        items: string[];
        maxItems?: number;
    } = $props();
</script>

<!-- `ctx.GhostButton` is typed `Component<…>` — a non-nullable function.
     Pre-fix, tsgo fires TS2774 ("this condition will always return true
     since this function is always defined") on the `ctx.GhostButton`
     operand because the synthesized `__svn_tpl_check` body only reads
     it via `typeof GhostButton` (type-position) and never as a value.
     Post-fix, emit writes `void [editable, ctx.GhostButton, items.length, maxItems];`
     at the top of the if-body so TS sees every condition operand as a
     value-position identifier reference inside the block. -->
{#if editable && ctx.GhostButton && items.length < maxItems}
    {@const GhostButton = ctx.GhostButton}
    <GhostButton label="add" />
{/if}

<!-- A plain function-import as a terminal `&&` operand — same TS2774
     shape, different trigger. `imgFn` resolves to `(src: string) => string`
     which is always-truthy; without the void-array marker the condition
     fires TS2774 on `imgFn`. -->
{#if items.length > 0 && imgFn}
    <Ghost label={imgFn(items[0])} />
{/if}
