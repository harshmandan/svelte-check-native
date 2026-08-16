<script lang="ts">
    // Used ONLY as aria attribute values. Dropping aria-* from the
    // emit left these unreferenced, firing TS6133 under
    // noUnusedLocals on identifiers upstream considers used.
    const label: string = 'row label';
    const rowIndex: number = 3;
</script>

<div role="row" aria-label={label} aria-rowindex={rowIndex}>x</div>
