<script lang="ts">
    import Child from './Child.svelte'
    const rows = [
        { id: 'a', label: 'first', meta: { kind: 'header' } },
        { id: 'b', label: 'second', meta: { kind: 'body' } },
    ]
</script>

<Child {rows} let:label let:kind>
    <p>{label.toUpperCase()}</p>
    <span>{kind.padStart(8)}</span>
    <!-- Wrong-method on `kind`: `padStart` exists, but `kind` was
         resolved as the meta.kind type — `string`. Calling a
         number-only method like `.toFixed()` on a string fails. -->
    <span>{kind.toFixed(2)}</span>
</Child>
