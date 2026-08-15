<script lang="ts">
    import type { RouteParams } from './$types';
    // The child's generated $types reaches its parent through a
    // sibling import('../$types.js') chain; the parent's matcher chain
    // then narrows `view` to 'grid' | 'list'. Assigning it to a number
    // is an error — unless either hop dangled, in which case the param
    // silently widened to `any` and nothing is reported.
    const bad: number = (null as unknown as RouteParams).view;
</script>

<p>{bad}</p>
