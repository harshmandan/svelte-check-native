<script lang="ts">
export let label = '';
const obj = { first: 1, second: 'x' };
const { first, second } = obj;
export { first, second };
</script>

<span>{label}</span>
