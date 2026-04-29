<svelte:options runes />
<script lang="ts">
    // Reviewer follow-up #4: unquoted component attrs with `{…}`
    // interpolations (`label=hi{bar}hi`) used to parse as a single
    // Text part — the parser explicitly stopped at whitespace/`>`/`/`
    // and never recognized `{` as an interpolation boundary. Pre-fix
    // the literal substring `hi{bar}hi` flowed through as a fixed
    // string and the embedded expression was never type-checked.
    //
    // Post-fix the unquoted-value parser mirrors the quoted-value
    // parser: flushes Text on `{`, parses the mustache body via the
    // shared brace-balancing scanner, pushes an Expression part.
    // The PropShape is now `TemplateLiteral` and the embedded
    // expression gets contextual typing. Mirrors upstream
    // svelte2tsx's `Attribute.ts:233`.
    import Child from './Child.svelte'

    const sym = Symbol('s')
    void sym
</script>

<!-- Unquoted value with a `{…}` interpolation. Symbol fails
     implicit string conversion (TS2731) at the inner expression
     position. -->
<Child label=boom-{sym}-end />
