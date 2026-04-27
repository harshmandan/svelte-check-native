<script lang="ts">
    // {#key EXPR} re-creates the body when EXPR changes. EXPR must
    // type-check like any other interpolation expression — a typo
    // or out-of-scope reference should fire TS2304/TS2552. Pre-fix
    // we walked the body but ignored the expression entirely.
    let item = $state({ id: 'a' });
    void item;
</script>

{#key item.id}
    <p>defined: {item.id}</p>
{/key}

{#key undefinedFoo}
    <p>this should fire</p>
{/key}
