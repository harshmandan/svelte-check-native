<svelte:options runes />
<script lang="ts">
    // Reviewer follow-up #9: multi-part component attrs (a quoted
    // attribute value with one or more `{…}` interpolations) used to
    // emit as `name: __svn_any()` — the prop key was present in the
    // satisfies object so the rest of the component's checks stayed
    // alive, but the embedded expressions were never type-checked
    // and the prop's value carried `any` instead of a real string.
    //
    // Post-fix the emit is a TS template literal `\`a ${b} c\``,
    // matching upstream svelte2tsx's `Attribute.ts:233`. Embedded
    // expressions get contextual typing (each `${…}` slot must be
    // string-coercible — anything not implicitly stringifiable like
    // `Symbol(…)` fails, but most types pass), AND the prop's
    // resulting type is `string` so a Child declaring
    // `label: number` would now mismatch.
    import Child from './Child.svelte'

    const obj = { foo: 1 }
    void obj

    const sym = Symbol('s')
    void sym
</script>

<!-- Clean: `string` interpolated inside a string-typed prop slot. -->
<Child label="prefix-{obj.foo}-suffix" />

<!-- Wrong: `Symbol(…)` is NOT implicitly string-coercible. Template
     literals reject it via TS2731 / TS2322. Pre-fix the whole prop
     was `__svn_any()` and the symbol passed silently. -->
<Child label="boom-{sym}-end" />
