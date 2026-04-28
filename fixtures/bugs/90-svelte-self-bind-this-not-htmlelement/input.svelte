<svelte:options runes />
<script lang="ts">
    // Reviewer item #1a: `bind:this` on `<svelte:self>` /
    // `<svelte:component>` is a COMPONENT instance ref, not an
    // HTMLElement. Pre-fix the DOM-element bind:this check fired
    // here, wrapping the target with `__svn_bind_this_check<…>`
    // which expected an HTMLElement-compatible value. The
    // component-instance type (e.g. `let me: any`) doesn't
    // satisfy that, so a spurious TS2322 fired at every
    // `<svelte:self bind:this={…}>` site. The full
    // component-instantiation port (#1b) is deferred — for now
    // we just stop emitting the wrong DOM-element check.
    let me: any = null
</script>

<svelte:self bind:this={me} />
