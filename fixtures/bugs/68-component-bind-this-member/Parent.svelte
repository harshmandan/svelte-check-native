<script lang="ts">
    // v0.3 §1.5: component bind:this on member expressions.
    // Previously dropped at analyze; now emits `refs.child = $inst;`
    // which type-checks refs.child's declared type against the
    // component instance type.
    import Child from './Child.svelte'
    import type { ComponentProps } from 'svelte'

    const refs: { child?: import('svelte').Component<ComponentProps<typeof Child>> } = {}
</script>

<Child label="hi" bind:this={refs.child} />
