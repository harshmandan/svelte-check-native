<script lang="ts">
    // The leaf tsconfig restates `paths` without spreading the base's,
    // so `$lib/*` no longer exists and this import cannot resolve.
    import { n } from "$lib/util";
</script>

<p>{n}</p>
