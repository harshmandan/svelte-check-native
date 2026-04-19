<script lang="ts">
    // All element-native one-way bindings shipped in v0.3 Item 4.
    // Clean case: every target has a compatible declared type.
    let nw: number = 0
    let nh: number = 0
    let dur: number = 0
    let seek: boolean = false
    let ended: boolean = false
    let ready: number = 0
</script>

<img src="" alt="" bind:naturalWidth={nw} bind:naturalHeight={nh} />
<video bind:duration={dur} bind:seeking={seek} bind:ended={ended} bind:readyState={ready}>
    <track kind="captions" />
</video>
