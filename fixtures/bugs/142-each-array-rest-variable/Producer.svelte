<script lang="ts">
    // Round-11 follow-up #4: ArrayRest projection over a VARIABLE
    // array (`string[]`, not a tuple). Round-10 #4's conditional
    // `T extends readonly [unknown, ...infer R]` doesn't reliably
    // match variable arrays — TS treats the tuple pattern as a
    // fixed-length prefix and `string[]` falls through to `never`.
    // Round-11 #4 adds a fallback branch:
    //   T extends readonly (infer U)[] ? U[] : never
    // so variable arrays project to an array of element type.
    //
    // Producer here exposes `tail` typed as `string[]` from
    // `{#each items as [head, ...tail]}` over `string[][]`.
    let { rows }: { rows: string[][] } = $props()
</script>

{#each rows as [head, ...tail]}
    <slot {head} {tail} />
{/each}
