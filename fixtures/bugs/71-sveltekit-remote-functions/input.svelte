<script lang="ts">
    // Import from a `.remote.ts` file — these are SvelteKit's
    // experimental remote functions. svelte-check-native treats
    // them as plain TypeScript modules; type-checking the import +
    // call should "just work" because the bundler resolves the
    // explicit `.remote.ts` extension and tsgo follows the export
    // signatures.
    import { getUser, getCount } from './data.remote.ts';

    const user = await getUser('1');
    const count = await getCount();

    // Type narrowing: `user.name` is `string`, `user.missing` would
    // be a TS2339 error — this asserts the cross-file types flow.
    const greeting: string = `Hello, ${user.name}!`;
    const total: number = count + 1;
</script>

<p>{greeting}</p>
<p>total: {total}</p>
