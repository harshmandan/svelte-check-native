<script lang="ts">
    // v0.3 Item 7: bind:this on BOTH simple identifiers and member
    // expressions. All these are compatible with their element tags.
    let divEl: HTMLDivElement | null = null
    let inputEl: HTMLInputElement | null = null
    const refs: { video?: HTMLVideoElement } = {}
    const els: HTMLElement[] = []
</script>

<!-- Simple identifier, exact tag type — clean. -->
<div bind:this={divEl}></div>

<!-- Simple identifier, broader declared type — accepts specific tag. -->
<input bind:this={inputEl} />

<!-- Member expression — previously unsupported at type-check layer. -->
<video bind:this={refs.video}>
    <track kind="captions" />
</video>

<!-- Array-index member expression. -->
{#each [0, 1] as i}
    <span bind:this={els[i]}></span>
{/each}
