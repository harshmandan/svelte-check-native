/// <reference types="./svn_shims.d.ts" />

// Pattern 3 — default-export declaration.
//
// TS form was:
//   declare const __svn_component_default: import('svelte').Component<P>;
//   export default __svn_component_default;
//
// `declare const` is TS-only syntax (no runtime form). JS-overlay
// rewrite: a regular runtime const annotated via JSDoc @type. The
// `null` cast is fine — consumers only inspect the type, never call
// the value.

/** @type {import('svelte').Component<{ value?: string }> | null} */
const __svn_component_default = null;
export default __svn_component_default;
