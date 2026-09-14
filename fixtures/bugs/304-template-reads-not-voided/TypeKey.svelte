<script lang="ts">
  import type { AppVideo } from './types';
  function fn(o: Record<string, unknown>) { return JSON.stringify(o); }
</script>
<p>{fn({ AppVideo: 1 })}</p>
