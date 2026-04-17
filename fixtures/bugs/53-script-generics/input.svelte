<script generics="T extends { id: string }" lang="ts">
    // Props type references the component's generic parameter. Before
    // the `generics` attribute was threaded through the emit, the
    // `type Props` declaration got hoisted out of $$render into the
    // overlay's module scope where `T` was unbound, producing
    // "Cannot find name 'T'" against every property that referenced it.
    type Props = { item: T; label: string };
    let { item, label }: Props = $props();
</script>

<p>{label}: {item.id}</p>
