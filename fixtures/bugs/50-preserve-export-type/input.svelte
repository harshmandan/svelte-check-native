<script lang="ts">
    import Panel from './Panel.svelte';
    import type { Section, PanelConfig } from './Panel.svelte';

    // If the overlay for Panel.svelte drops the instance-script
    // `export type Section` / `export interface PanelConfig`, this
    // import-type resolves to nothing and the annotations below fail
    // to type-check (Section becomes undefined).
    const current: Section = 'a';
    const cfg: PanelConfig = { section: current, count: 3 };
</script>

<Panel config={cfg} />
