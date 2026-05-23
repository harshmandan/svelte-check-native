<script lang="ts">
    // Upstream svelte2tsx replaces HTML comments with `/* ... */` blocks
    // during overlay generation (language-tools#1828 — marked wontfix as
    // an architectural limitation). When the source HTML comment
    // contains `*/`, the synthesized block closes prematurely, breaking
    // the generated default export and every downstream import. Our
    // AST-based comment walker preserves the structure verbatim, so
    // this import resolves cleanly.
    import Producer from './Producer.svelte'
</script>

<Producer />
