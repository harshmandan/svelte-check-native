<script lang="ts">
    // v0.3 §1.3: after narrowing denarrow to only null/undefined
    // initializers, simple-identifier bind targets with non-narrow
    // inits (like `= 0` or `= ''`) are no longer widened to `any`
    // before __svn_tpl_check. The bind:value contract check now
    // fires on wrong-typed simple-identifier targets.
    let strForNumber: string = ''
</script>

<input type="number" bind:value={strForNumber} />
