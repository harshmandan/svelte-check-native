<script lang="ts">
    const known = 'k';
</script>

<slot
    name="header"
    tooltip="first line
second line"
    count={1 + 1}
    good="prefix {known} suffix"
/>
<slot name="labeled" label="prefix {missingName} suffix" />
