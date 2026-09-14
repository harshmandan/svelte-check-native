<script lang="ts">
    import Markup from './Markup.svelte';
    import Comment from './Comment.svelte';
    import Inspect from './Inspect.svelte';
    import Dispatcher from './Dispatcher.svelte';

    const a: InstanceType<typeof Markup> | null = null;
    const b: InstanceType<typeof Comment> | null = null;
    const c: InstanceType<typeof Inspect> | null = null;
    const d = new Dispatcher({ target: document.body });
    d.$on('bar', () => {});
</script>

<p>{a}{b}{c}</p>
