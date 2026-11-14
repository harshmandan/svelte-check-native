<script lang="ts">
    const obj = { a: 1 };
    const ok = "red";
</script>

<div style:color="{obj}">a</div>
<div style:color="{ok}">b</div>
<div style:color='{obj}'>c</div>
<div style:color={obj}>d</div>
