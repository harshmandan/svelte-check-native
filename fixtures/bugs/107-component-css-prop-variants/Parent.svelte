<svelte:options runes />
<script lang="ts">
    // Reviewer follow-up #5: CSS custom-property attrs (`--foo`) on a
    // component must always wrap through `__svn_css_prop({…})` so the
    // key isn't checked against the component's declared Props. Pre-fix
    // the wrap fired only for `Literal` and `Expression` PropShape
    // variants — `BoolShorthand` (`<Comp --foo>`) and `TemplateLiteral`
    // (`<Comp --foo="a {b}">`) fell through as regular component props
    // and hit excess-prop on the strict Props type.
    //
    // Post-fix all four variants wrap. Mirrors upstream
    // svelte2tsx's `Attribute.ts:97-107` byte-for-byte.
    import Child from './Child.svelte'

    const expr = 1
    void expr
</script>

<!-- Each row exercises a different variant; ALL must compile clean
     because they're treated as CSS props, not component props. -->
<Child --css-literal="hi" />
<Child --css-expression={expr} />
<Child --css-template="lit-{expr}-end" />
<Child --css-bool />
