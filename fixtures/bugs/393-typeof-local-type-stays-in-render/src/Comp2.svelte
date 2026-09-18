<script lang="ts">
const handler = (e: MouseEvent) => e.button;
type Handler = typeof handler;
let { cb }: { cb: Handler } = $props();
</script>
{cb}
