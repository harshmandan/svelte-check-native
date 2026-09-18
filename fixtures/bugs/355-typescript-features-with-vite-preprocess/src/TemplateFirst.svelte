<script module>
  export enum Color { Red }
</script>

<script lang="ts">
  let n: number = 1;
</script>

<p>{new (class { accessor v = n })().v}</p>
