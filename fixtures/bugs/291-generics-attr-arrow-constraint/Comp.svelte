<script lang="ts" generics="T extends () => void, U">
    interface Props { a: T; b: U; c: number }
    let { a, b, c }: Props = $props();
</script>

<p>{typeof a}{typeof b}{c}</p>
