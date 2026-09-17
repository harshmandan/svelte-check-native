<script lang="ts">
  let amount: number | null = $state(null);
  let type = 'number';
  let n: number = $state(0);
  let s: string = $state('');
  let sel: string = $state('');
  let ta: string = $state('');
  let num2: number = $state(0);
</script>
<input {type} bind:value={amount} />
<input bind:value={amount} />
<input type="number" bind:value={s} />
<input type="text" bind:value={n} />
<select bind:value={sel}><option>a</option></select>
<textarea bind:value={ta}></textarea>
<input type="range" bind:value={num2} />
<input type="checkbox" bind:value={s} />
