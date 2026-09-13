<script lang="ts">
    import { writable } from 'svelte/store';

    const selected = writable(false)
    export let label: string = ''
    export let checklist: boolean | undefined = undefined
    let _class: any = ''
    export { _class as class }

    $: if (checklist !== undefined) {
        selected.set(checklist)
    }
</script>

<div class={_class}>{label} {$selected}</div>
