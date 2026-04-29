<script lang="ts">
    // Child has a named slot "header" exposing a `title` prop.
    export let label: string = ''
</script>

<slot name="header" title="hi">{label}</slot>
