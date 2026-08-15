<script lang="ts">
    // util/ lives beside the base config that declares the alias, not
    // beside this entry config — resolving proves the anchor.
    import { n } from "$util/thing";
    const m: number = n;
</script>

<p>{m}</p>
