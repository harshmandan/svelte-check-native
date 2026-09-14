<script lang="ts">
    type T = $$Generic<Record<string, () => void>>;
    export let x: T;
</script>

<p>{Object.keys(x).length}</p>
