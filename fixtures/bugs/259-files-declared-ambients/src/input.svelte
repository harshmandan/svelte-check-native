<script lang="ts">
    // MY_GLOBAL is declared in extra/ambient.d.ts, which the tsconfig
    // pulls in through `files` rather than `include`.
    const w: number = MY_GLOBAL;
</script>

<p>{w}</p>
