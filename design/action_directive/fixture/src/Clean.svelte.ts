// Clean case: destructure uses CORRECT SubmitFunction param names.
// Expected tsgo result: 0 errors.
//
// This is what our emit should produce for a Svelte source like:
//
//   <script>
//     import { enhance } from '$app/forms';
//   </script>
//   <form use:enhance={({ formData, formElement }) => {
//       return async ({ result }) => { console.log(result); };
//   }}>
//       <button>Go</button>
//   </form>

import { enhance } from './app_forms.ts';

async function $$render() {
    async function __svn_tpl_check() {
        {
            const __svn_action_0 = __svn_ensure_action(
                enhance(__svn_map_element_tag('form'), (({ formData, formElement }) => {
                    void formData;
                    void formElement;
                    return async ({ result }) => {
                        void result;
                    };
                })),
            );
            void __svn_action_0;
        }
    }
    void __svn_tpl_check;
}
void $$render;
