<script lang="ts">
    // Inner declares a typed event surface — the parent's `on:click`
    // bubble must surface this exact shape on the wrapper's own
    // `$$Events`.
    interface $$Events {
        click: CustomEvent<{ id: number }>
    }
    interface $$Props {
        label?: string
    }
    let _e: $$Events | undefined
    void _e
    export let label: string = ''
</script>

<button>{label}</button>
