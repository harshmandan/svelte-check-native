<script lang="ts">
  let a = $state(1);
  enum K { X }
  let b = a;
</script>
<p>{b}{K.X}</p>
