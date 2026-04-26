<script lang="ts">
    import { dev } from '$app/environment';
    import { page } from '$app/state';
</script>

{dev}
{page.url.pathname}
