<script lang="ts">
    let selected = $state.raw<ReadonlySet<string>>(new Set())
    let previous: ReadonlySet<string> | null = $state.raw(null)

    function select(value: string) {
        previous = selected
        selected = new Set([value])
    }
</script>

<button onclick={() => select('alpha')}>
    {selected.has('alpha') ? 'Selected' : 'Select'}
    {previous?.has('alpha') ? '(previously selected)' : ''}
</button>
