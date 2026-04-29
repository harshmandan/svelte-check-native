<script lang="ts">
    // Inner exposes `tooltip` via its default slot. Svelte-4
    // class-style so the iso-component shape's `S` parameter
    // carries the slot type (extracted by `__SvnComponentSlots`).
    export let label: string = ''
    void label
</script>

<slot tooltip={{ x: 0, y: 0 }}>{label}</slot>
