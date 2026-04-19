<script lang="ts">
    // Wrong target types per bind:value dispatch on MEMBER
    // expressions. Simple-identifier targets (`let num: number = 0;
    // <input type="number" bind:value={num}>`) skip the check because
    // our definite-assign rewrite (`num = undefined as any;`) widens
    // their flow type to `any` before __svn_tpl_check runs.
    // Documented as a limitation in NEXT.md.
    //
    // Member expressions skip the definite-assign rewrite (no simple
    // declaration to target), so the check fires.
    const state: {
        strForNumber?: string
        numForText?: number
        numForTextarea?: number
    } = {}
</script>

<input type="number" bind:value={state.strForNumber} />
<input type="text" bind:value={state.numForText} />
<textarea bind:value={state.numForTextarea}></textarea>
