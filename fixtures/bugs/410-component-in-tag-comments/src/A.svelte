<script lang="ts">import Comp from './Comp.svelte';</script>
<Comp
  // @ts-expect-error
  bad={1} />
<Comp
  // @ts-expect-error
  title="x" />
<Comp title="y" /* keep */ on:x={() => {}}
  // @ts-expect-error
  nope={2} />
<Comp title="z"
  // @ts-expect-error
/>
