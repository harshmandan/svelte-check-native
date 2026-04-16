<script lang="ts">
    let inputEl: HTMLDivElement;

    function focusLater() {
        setTimeout(() => inputEl.focus(), 100);
    }

    focusLater();
</script>

<div bind:this={inputEl}></div>
