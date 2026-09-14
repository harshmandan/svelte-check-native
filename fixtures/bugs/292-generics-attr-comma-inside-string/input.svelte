<script lang="ts" generics="T extends 'a,b', U">
    interface Props { a: T; b: U }
    let { a, b }: Props = $props();
</script>

<p>{a}{typeof b}</p>
