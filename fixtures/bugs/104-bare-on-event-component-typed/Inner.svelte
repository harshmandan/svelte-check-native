<svelte:options runes />
<script lang="ts">
    // Inner component declares a typed event surface via interface
    // $$Events. Only "click" and "submit" are valid event names; any
    // other name on a consumer's `on:NAME` directive should fail.
    interface $$Events {
        click: CustomEvent<{ x: number }>
        submit: CustomEvent<{ y: number }>
    }
    interface $$Props {
        label?: string
    }

    let { label = '' }: $$Props = $props()
    let _events: $$Events | undefined
    void _events
</script>

<button>{label}</button>
