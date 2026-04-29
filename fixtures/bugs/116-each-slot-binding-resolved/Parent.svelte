<script lang="ts">
    import Child from './Child.svelte'
    const rows = [
        { id: 'a', label: 'first' },
        { id: 'b', label: 'second' },
    ]
</script>

<!-- Consumer destructures the typed slot props. The resolved
     `item` carries `{ id: string; label: string }`; the resolved
     `index` carries `number`. -->
<Child items={rows} let:item let:index>
    <span>{index}: {item.id}</span>
    <!-- Wrong-field access: `item.title` doesn't exist on the row
         shape. Post-fix this fires TS2339 because the slot binding
         was resolved through the each-block's `items` expression.
         Pre-fix this passed silently because the slot-def dropped
         `item` entirely (no narrowing flowed). -->
    <span>{item.title}</span>
</Child>
