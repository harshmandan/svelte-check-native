<script lang="ts">
    let {
        checked,
        onchange,
    }: {
        checked: boolean;
        onchange: (event: { checked: boolean }) => void;
    } = $props();
</script>

<button onclick={() => onchange({ checked: !checked })}>{checked ? 'on' : 'off'}</button>
