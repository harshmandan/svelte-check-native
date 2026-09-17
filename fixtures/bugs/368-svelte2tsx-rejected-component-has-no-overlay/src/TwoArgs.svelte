<script lang="ts">
  type A = string;
  type B = number;
  type T = $$Generic<A, B>;
  export let v: T;
</script>
{v}
