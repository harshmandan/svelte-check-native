<script lang="ts">
    // +error.svelte with an untyped $props() destructure. Kit renders
    // the nearest error boundary with the failing `error` as a prop,
    // typed app-wide by the ambient `App.Error` interface. Our emit
    // recognizes the route basename and injects `error: App.Error`
    // into the synthesized default-export Props shape; other
    // destructured names stay `any` (only `error` is Kit-injected on
    // error routes).
    let { error } = $props();
</script>

<h1>Something went wrong</h1>
<p>{error.message}</p>
