<script lang="ts">
    type Size = 'small' | 'large';

    // A long string split across lines with the operator at the end of
    // each line. Nothing may be inserted before the expression finishes.
    export let label: string =
        'first part, ' +
        'second part';

    export let size: Size = 'small';

    const isLarge = size === 'large';
</script>

<p class:large={isLarge}>{label}</p>
