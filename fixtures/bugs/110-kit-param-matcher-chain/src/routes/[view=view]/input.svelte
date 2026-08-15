<script lang="ts">
    import type { RouteParams } from './$types';
    // The matcher narrows `view` to 'grid' | 'list'. Assigning it to a
    // number is an error — unless the matcher chain dangled, in which
    // case the param silently widened to `any` and nothing is reported.
    const bad: number = (null as unknown as RouteParams).view;
</script>

<p>{bad}</p>
