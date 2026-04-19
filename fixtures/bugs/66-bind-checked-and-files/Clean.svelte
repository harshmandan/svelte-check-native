<script lang="ts">
    // v0.3 Item 8 (narrow): bind:checked and bind:files on <input>
    // get `boolean` and `FileList | null` target checks respectively.
    // Both are tag-independent of the input's `type` attribute.
    let isAgreed: boolean = false
    let uploadedFiles: FileList | null = null
</script>

<input type="checkbox" bind:checked={isAgreed} />
<input type="file" bind:files={uploadedFiles} />
