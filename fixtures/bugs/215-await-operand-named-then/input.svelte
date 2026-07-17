<script lang="ts">
    const then_: Promise<string> = Promise.resolve('a');
    const fallback: Promise<string> = Promise.resolve('b');
    // A variable literally named `then` in operand position.
    const then = then_;
    let flag = true;
</script>

{#await flag ? then : fallback}
    <p>waiting</p>
{:then value}
    <p>{value.toUpperCase()}</p>
{/await}
