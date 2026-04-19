<script lang="ts">
    // v0.3 Item 8 extended: bind:value dispatches per element tag and
    // literal `type="..."` attribute.
    //   <input type="number" bind:value={num}>  → num must be number
    //   <input type="range"  bind:value={rng}>  → rng must be number
    //   <input type="text"   bind:value={txt}>  → txt must be string
    //   <input               bind:value={def}>  → def must be string (default)
    //   <textarea bind:value={para}>            → para must be string
    let num: number = 0
    let rng: number = 0
    let txt: string = ''
    let def: string = ''
    let para: string = ''
</script>

<input type="number" bind:value={num} />
<input type="range" bind:value={rng} />
<input type="text" bind:value={txt} />
<input bind:value={def} />
<textarea bind:value={para}></textarea>
