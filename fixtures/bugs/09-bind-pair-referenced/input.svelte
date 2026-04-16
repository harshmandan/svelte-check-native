<script lang="ts">
    let internal = '';
    function getValue() {
        return internal;
    }
    function setValue(v: string) {
        internal = v;
    }
</script>

<input bind:value={() => getValue(), (v: string) => setValue(v)} />
