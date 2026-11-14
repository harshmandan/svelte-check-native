<script lang="ts">
    let count: number = "one";
    let id = "a";
    let props = { title: "t" };
    let value = "";
</script>

<div class= "x" title ="y">{count}</div>
<div {id }></div>
<div { ...props}></div>
<input bind:value= {value} />
