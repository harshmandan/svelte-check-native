<script lang="ts">
    import { writable } from 'svelte/store';
    const count = writable(0);
    const quote = /'/; const v: number = $count;
    const tick = /`/;
    const w: number = $count;
</script>

<p>{quote}{tick}{v}{w}</p>
