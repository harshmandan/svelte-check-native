<script lang="ts">
// This component does NOT use createEventDispatcher. The mention
// in this comment should not flip the default-export shape away
// from `__sveltets_2_fn_component`. Pre-fix the substring scan
// matched the comment and forced the iso shape, which then
// surfaced TS2322 false-positives at consumer sites doing
// `Parameters<typeof Comp>` or `(typeof Comp)[]`.

let { value }: { value: number } = $props()
</script>

<span>{value}</span>
