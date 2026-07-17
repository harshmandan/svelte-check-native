<script lang="ts">
    let plain = '';
    function pick<A, B>(v: A, _b?: B): A { return v; }
    function sink(v: number) { plain = String(v); }
</script>

<input bind:value={() => pick<string, number>(plain), (v) => sink(v)} />
<p>{plain}</p>
