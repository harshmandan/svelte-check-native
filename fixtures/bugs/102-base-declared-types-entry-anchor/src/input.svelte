<script lang="ts">
    // AMB comes from typedefs/amb.d.ts, pulled in by a `types` entry that
    // the BASE config declares as "./typedefs/amb.d.ts" — a path that only
    // resolves from the entry tsconfig's directory, not the base's.
    const v: string = AMB;
</script>

<p>{v}</p>
