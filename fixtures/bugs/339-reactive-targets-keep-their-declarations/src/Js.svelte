<script>
  export let count;
  let doubled;
  console.log(doubled.toFixed());
  $: doubled = count * 2;
  $: ({ a, b } = { a: 1, b: count });
</script>
{doubled}{a}{b}
