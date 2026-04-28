<script lang="ts">
    import Child from './Child.svelte'
</script>

<!-- The `let:kind` destructure must resolve `kind` against the slot
     def's literal entry (`kind: "header"`) so consumers can read the
     value. -->
<Child let:kind>
    <span>{kind}</span>
</Child>
