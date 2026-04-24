// Simulated overlay: <script lang="ts"> ComponentPreview.svelte that
// writes `let { id, code = {}, view = 'small', loading = false, append = '' } = $props()`
// with NO TS annotation on the destructure.
//
// Upstream svelte2tsx synthesises `type $$ComponentProps = { id: any,
// code?: Record<string, any>, view?: string, loading?: boolean,
// append?: string }` and uses it as the Props. The user's JSDoc
// `/** @typedef {Object} Props */` typedef is IGNORED because
// isTsFile=true (see ExportedNames.ts "Hard mode" `if (!this.isTsFile)`).
//
// Critical: `head` is in the JSDoc Props but NOT in the destructure.
// So the synthesised $$ComponentProps has no `head` — a consumer
// passing `head={…}` fires TS2353 against $$ComponentProps.

type $$ComponentProps = {
    id: any,
    code?: Record<string, any>,
    view?: string,
    loading?: boolean,
    append?: string,
};

declare const __svn_component_default: import('svelte').Component<$$ComponentProps>;
declare type __svn_component_default = import('svelte').SvelteComponent<$$ComponentProps>;
export default __svn_component_default;
