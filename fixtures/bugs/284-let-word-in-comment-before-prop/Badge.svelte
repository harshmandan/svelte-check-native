<script lang="ts">
    // let the parent pick a class for the <span
    /* We let consumers decide (e.g. "big" */
    export let xyz: string;
</script>

<span class="{xyz}"><slot /></span>
