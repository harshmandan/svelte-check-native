<script lang="ts">
    let x: number = 1;
    const y: number = "x";
</script>
<div bind:noAssignment={x} foo={1}></div>
<p>{x}{y}</p>
