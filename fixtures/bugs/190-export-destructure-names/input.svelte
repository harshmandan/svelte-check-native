<script lang="ts">
    const obj = { a: 1, b: 2, c: 3 };
    const arr = [10, 20, 30];
    // Destructured exports: every leaf name must surface in the
    // component's Exports intersection (upstream addExportForBindingPattern).
    export const { a, b } = obj;
    export const { c, ...rest } = obj;
    export let [x, , z] = arr;
</script>

{a}{b}{c}{x}{z}
