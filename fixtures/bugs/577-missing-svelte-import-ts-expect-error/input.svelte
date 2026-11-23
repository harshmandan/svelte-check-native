<script lang="ts">
    // @ts-expect-error - suppressed missing named import
    import { missingA } from './MissingA.svelte';
    // @ts-ignore - ignored missing default import
    import MissingB from './MissingB.svelte';
    /* @ts-expect-error */
    import './MissingSideEffect.svelte';
    import MissingC from './MissingC.svelte';
    // @ts-expect-error - unused because Real.svelte exists
    import Real from './src/Real.svelte';

    export const out = [missingA, MissingB, MissingC, Real];
</script>

<Real />
