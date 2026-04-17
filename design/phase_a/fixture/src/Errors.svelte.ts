// Simulated overlay emit for Errors.svelte — deliberate type errors the
// new emit shape MUST catch. Each `// EXPECT: TS####` comment is what
// tsgo should report at that line.
//
// Source Svelte (conceptual):
//   <script lang="ts">
//       import Switch from './Switch.svelte';
//       import Wrapper from './Wrapper.svelte';
//   </script>
//   <!-- Error 1: wrong prop type (checked must be boolean) -->
//   <Switch checked="not-a-boolean" onchange={() => {}} />
//   <!-- Error 2: destructure non-existent field from callback -->
//   <Switch checked={true} onchange={({ nope }) => nope} />
//   <!-- Error 3: excess prop -->
//   <Switch checked={true} onchange={() => {}} foo="bar" />
//   <!-- Error 4: snippet param destructure includes non-existent field -->
//   <Wrapper items={[]}>
//       {#snippet row({ id, label, missing })}
//           <td>{id}: {missing}</td>
//       {/snippet}
//   </Wrapper>

import Switch from './Switch.svelte.ts';
import Wrapper from './Wrapper.svelte.ts';

async function $$render_errors() {
    async function __svn_tpl_check() {
        // Error 1: TS2322 — string not assignable to boolean.
        Switch(__svn_any(), {
            checked: 'not-a-boolean',
            onchange: () => {},
        });

        // Error 2: TS2339 — property 'nope' does not exist on type '{ checked: boolean }'.
        Switch(__svn_any(), {
            checked: true,
            onchange: ({ nope }) => nope,
        });

        // Error 3: TS2353 — object literal may only specify known properties,
        //                   'foo' does not exist in type '{ checked, onchange }'.
        Switch(__svn_any(), {
            checked: true,
            onchange: () => {},
            foo: 'bar',
        });

        // Error 4: TS2339 — property 'missing' does not exist on
        //                   '{ id: number; label: string }'.
        Wrapper(__svn_any(), {
            items: [],
            row: ({ id, label, missing }) => {
                void id;
                void label;
                void missing;
                return __svn_snippet_return();
            },
        });
    }
    void __svn_tpl_check;
}
$$render_errors;

declare const __svn_component_default: any;
export default __svn_component_default;
