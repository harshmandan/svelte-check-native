<script lang="ts">
    // MY_GLOBAL is declared by the `myglobals` package under the
    // project's own typeRoots, not under node_modules/@types.
    const v: string = MY_GLOBAL;
</script>

<p>{v}</p>
