// Wrong case: destructure uses names that DON'T exist on SubmitFunction's
// parameter type. Expected tsgo result: exactly 3 TS2339 errors
// ("Property 'X' does not exist on type …") on form / data / submit.
//
// This mirrors the user-reported bug: `use:enhance={({form, data, submit}) => ...}`
// — the user confused these with the `$props()` destructure names.
//
// Source shape:
//   <script>
//     import { enhance } from '$app/forms';
//   </script>
//   <form use:enhance={({ form, data, submit }) => { ... }}>

import { enhance } from './app_forms.ts';

async function $$render() {
    async function __svn_tpl_check() {
        {
            const __svn_action_0 = __svn_ensure_action(
                enhance(__svn_map_element_tag('form'), (({ form, data, submit }) => {
                    void form;
                    void data;
                    void submit;
                })),
            );
            void __svn_action_0;
        }
    }
    void __svn_tpl_check;
}
void $$render;
