<script lang="ts">
    // Round-15 follow-up #4: destructure-default handling switched
    // from a type-level approximation
    // (`Exclude<T, undefined> | <typeof default>`) to upstream's
    // VALUE-level IIFE shape (`slot.ts:117`):
    //   `((PATTERN) => leaf)(undefined as any as ELEMENT_TYPE)`.
    //
    // The approximation worked for simple defaults
    // (literals / bare idents) but lost fidelity for object /
    // array / template-literal defaults. The hardest break: a
    // template-literal default with a value-level interpolation
    // (`\`prefix-${id}\``) was passed through into a TS TYPE
    // position — TypeScript template literal types REQUIRE type-
    // level interpolations, so a value identifier (`id`) inside
    // `${…}` produced TS2304 (`Cannot find name 'id'`) on the
    // emitted overlay.
    //
    // Post-fix the IIFE places the destructure pattern back at
    // VALUE level. TS evaluates the default normally — value-name
    // interpolations resolve through closure; object / array
    // defaults preserve their precise types.
    type Row = { value?: undefined }
    let { rows }: { rows: Row[] } = $props()

    // Module-scope value referenced from the default's template
    // literal interpolation. Pre-fix this leaked into a TS type
    // position and fired TS2304 on the emitted overlay.
    const PREFIX = 'p-'
</script>

{#each rows as { value = `${PREFIX}fallback` }}
    <slot v={value} />
{/each}
