<script lang="ts">
  export let a = 1;
  export let b = 2;
  $: void (a, b);
  $: (a, b);
  $: { void (a, b); console.log('x'); }
</script>
<p>{a}{b}</p>
