<script lang="ts" generics="T">
    // Svelte-4 generic component with a slot prop typed `T`. Upstream
    // svelte2tsx's old emit lost T at the slot consumer
    // (language-tools#1344 — marked wontfix). Our Rule-#7
    // `new $$_CN({ target, props })` instantiation pattern preserves
    // T at the slot-let binding site so the parent's `let:values`
    // sees the concrete type instead of `unknown`.
    export let values: T
</script>

<slot {values} />
