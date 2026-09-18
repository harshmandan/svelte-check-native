<script lang="ts">
  import Destructure from "./Destructure.svelte";
  import Arrow from "./Arrow.svelte";
</script>
<Destructure a={1} />
<Destructure b={2} />
<Arrow format={(n) => n} />
<Arrow parse={() => "x"} />
