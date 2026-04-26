<script lang="ts">
    // Svelte 5.36+ experimental async — three sites for `await`:
    //
    //   (1) top-level inside <script>
    //   (2) inside `$derived(...)` rune call
    //   (3) inline `{await ...}` in markup
    //
    // svelte-check-native's render fn is `async function $$render_…`
    // and the template-check wrapper is `async function __svn_tpl_check`,
    // so `await` is syntactically valid in all three sites without
    // any pipeline changes — this fixture locks that.
    import { fetchTitle, fetchCount } from './data.ts';

    // (1) top-level await
    const title = await fetchTitle();

    // (2) await inside $derived (Svelte 5 rune)
    const upper = $derived(await Promise.resolve(title.toUpperCase()));
</script>

<h1>{title}</h1>
<p>upper: {upper}</p>
<!-- (3) inline await in markup interpolation -->
<p>count: {await fetchCount()}</p>
