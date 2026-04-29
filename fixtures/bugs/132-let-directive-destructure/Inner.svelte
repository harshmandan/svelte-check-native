<script lang="ts">
    interface $$Slots {
        default: {
            tooltip: { x: number; y: number; label: string }
        }
    }
    let _s: $$Slots | undefined
    void _s
    let { children }: { children?: any } = $props()
    void children
</script>

<slot tooltip={{ x: 1, y: 2, label: 'a' }} />
