<script lang="ts">
    // One wrong-typed target per binding class. TS2345 must fire at
    // each `bind:NAME={...}` position.
    let nh: boolean = false
    let dur: string = ''
</script>

<img src="" alt="" bind:naturalHeight={nh} />
<video bind:duration={dur}>
    <track kind="captions" />
</video>
