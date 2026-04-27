<script lang="ts">
    // Threlte/theatre Declare.svelte pattern: the parent component
    // passes a prop literally named `props`, so `$props()` destructure
    // pulls a binding called `props`. Previously svn-lint fired a
    // false-positive `store_rune_conflict` here — "you're using `$props`
    // rune but there is a local binding called `props`" — even though
    // the user's `$props()` call is the rune itself, not a store sub of
    // a separate `props` variable. Verified against upstream
    // svelte/compiler 5.53.6 directly: this pattern produces ONLY a
    // `state_referenced_locally` warning, never `store_rune_conflict`.

    interface InnerProps {
        title: string;
    }

    let { props, title }: { props: InnerProps; title: string } = $props();

    void props;
    void title;
</script>

<p>{title}</p>
