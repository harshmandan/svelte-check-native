<script lang="ts">
  let count = 0;
</script>
<p>{(new (class { #count = 1; get v() { return this.#count; } })()).v}</p>
