<script lang="ts">
  function f() { $: x = 1; }
  f();
  let y = 1;
  $: z = y * 2;
  $: console.log(z);
  $: if (y) { console.log(y); }
</script>
