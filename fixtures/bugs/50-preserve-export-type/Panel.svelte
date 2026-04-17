<script lang="ts">
    // `export type` at the instance-script level is pure type surface. It
    // must survive to the module top level of the generated overlay so
    // consumers writing `import type { Section } from './Panel.svelte'`
    // resolve. The `export interface` branch is the same shape.
    export type Section = 'a' | 'b' | 'c';
    export interface PanelConfig {
        section: Section;
        count: number;
    }

    let {
        config,
    }: {
        config: PanelConfig;
    } = $props();
</script>

<p>{config.section} — {config.count}</p>
