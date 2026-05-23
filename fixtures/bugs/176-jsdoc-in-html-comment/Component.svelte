<!--
/**
 * JSDoc comment block placed inside a Svelte/HTML comment. In
 * upstream svelte2tsx's string-based comment handling
 * (language-tools#2995), the nested `*/` here closes the OUTER JSDoc
 * block emitted into the generated overlay; tsgo then sees TS1109/
 * TS1161 on the broken JSDoc and abandons whole-program type-checking,
 * silently masking every real error in the workspace.
 *
 * Our AST-based comment handling skips Svelte/HTML comment regions
 * cleanly so the JSDoc-looking content is never reinterpreted as a
 * real JSDoc block. The script error below must still fire — proof
 * that diagnostics are not silently suppressed.
 *
 * @param {string} foo
 */
-->
<script lang="ts">
    // Deliberate type mismatch. If we were affected by upstream's bug
    // this error would vanish (along with every other error in the
    // workspace). Locking the win.
    const s: string = 42 as any as number
    void s
</script>
