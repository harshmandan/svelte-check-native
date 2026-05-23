<script lang="ts">
    import Form from './Form.svelte'

    interface Item {
        name: string
    }
    const initial: Item = { name: 'a' }
</script>

<!-- OK: values narrows to Item; .name is on the type. -->
<Form values={initial} let:values>
    {values.name}
</Form>

<!-- BAD: .doesntExist must fire TS2339 — proves the narrowing is
     real and not a fallback-to-any. -->
<Form values={initial} let:values>
    {values.doesntExist}
</Form>
