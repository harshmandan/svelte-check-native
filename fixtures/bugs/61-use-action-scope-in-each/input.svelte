<script lang="ts">
    type Action<N, T> = (node: N, params?: T) => {
        destroy?: () => void
    }
    const intersection: Action<HTMLElement, { cb: (n: number) => void }> = () => ({})
    const items = ['a', 'b', 'c']
    let active = 0
</script>

{#each items as item, index (index)}
    {@const label = `slide-${index}`}
    <div
        use:intersection={{
            cb: (_n) => {
                active = index
                void label
            },
        }}>
        {item}
    </div>
{/each}
