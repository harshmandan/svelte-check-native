<script lang="ts">
    const s = 'from "../ext/x"' as const;
    const t: 'from \"../ext/x\"' = s;
    const u = `import "../ext/x"` as const;
    const w: `import \"../ext/x\"` = u;
</script>

<p>{t}{w}</p>
