<script lang="ts">
    // MY_SCOPED_GLOBAL is declared by the scoped package `@myscope/mypkg`,
    // which TypeScript looks up under the node_modules/@types typeRoot as
    // the mangled directory `myscope__mypkg`.
    const v: string = MY_SCOPED_GLOBAL;
</script>

<p>{v}</p>
