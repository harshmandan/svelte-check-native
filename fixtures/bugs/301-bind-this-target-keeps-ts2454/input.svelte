<script lang="ts">
    let el: HTMLDivElement;
    const read = () => el.id;
    console.log(read(), el);
</script>

<div bind:this={el}></div>
