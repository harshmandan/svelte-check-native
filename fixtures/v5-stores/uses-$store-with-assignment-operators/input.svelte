<script>
  import { writable } from 'svelte/store';
  const count = writable(0);
  let myvar = 42 // to show that this is different from ++ or --
  const handler1 = () => $count += myvar
  const handler2 = () => $count -= myvar
  const handler3 = () => $count *= myvar
  const handler4 = () => $count /= myvar
  const handler5 = () => $count **= myvar
  const handler6 = () => $count %= myvar
  const handler7 = () => $count <<= myvar
  const handler8 = () => $count >>= myvar
  const handler9 = () => $count >>>= myvar
  const handler10 = () => $count &= myvar
  const handler11 = () => $count ^= myvar
  const handler12 = () => $count |= myvar
</script>

<button onclick={() => $count += myvar}>add</button>
<button onclick={() => $count -= myvar}>subtract</button>
<button onclick={() => $count *= myvar}>multiply</button>
<button onclick={() => $count /= myvar}>divide</button>
<button onclick={() => $count **= myvar}>exponent</button>
<button onclick={() => $count %= myvar}>mod</button>
<button onclick={() => $count <<= myvar}>leftshift</button>
<button onclick={() => $count >>= myvar}>rightshift</button>
<button onclick={() => $count >>>= myvar}>unsigned rightshift</button>
<button onclick={() => $count &= myvar}>AND</button>
<button onclick={() => $count ^= myvar}>XOR</button>
<button onclick={() => $count |= myvar}>OR</button>
