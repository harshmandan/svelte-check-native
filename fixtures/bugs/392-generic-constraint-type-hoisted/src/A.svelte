<script lang="ts">
type Item = { id: number };
type T = $$Generic<Item>;
export let item: T;
export let list: Item[] = [];
</script>
{item.id}{list.length}
