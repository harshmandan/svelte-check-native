<script lang="ts">
export let label = '';
function validate() {
	return label.length > 0;
}
export { validate };
</script>

<span>{label}</span>
