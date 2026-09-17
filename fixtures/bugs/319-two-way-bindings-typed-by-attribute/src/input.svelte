<script lang="ts">
  let s: string = $state('');
  let f: string = $state('');
  let n: number = $state(0);
  let ih: number = $state(0);
  let pausedS: string = $state('');
</script>
<input type="checkbox" bind:checked={s} />
<input type="file" bind:files={f} />
<input bind:value={n} />
<div bind:innerHTML={ih} contenteditable="true"></div>
<video bind:paused={pausedS}></video>
<input type="checkbox" bind:checked={n, (v) => (n = v ? 1 : 0)} />
<input type="text" bind:value={s, (v) => (s = v)} />
