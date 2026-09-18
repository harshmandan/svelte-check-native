<script lang="ts">
export let cb: Handler;
const handler = (e: MouseEvent) => e.button;
type Handler = typeof handler;
</script>
{cb}
