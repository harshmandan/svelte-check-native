<script lang="ts">
    // A binding literally named `derived` makes `$derived.by(...)` a
    // store subscription of that binding (the identifier sits in a
    // member access, not the rune-decl call position). The alias
    // references `derived`, whose init references `$derived` — a
    // self-referential declaration, so TS7022 fires. Upstream
    // svelte2tsx emits the same subscription and fires the same
    // diagnostic at the same user position.
    const derived = $derived.by(() => 1);
    void derived;
</script>
