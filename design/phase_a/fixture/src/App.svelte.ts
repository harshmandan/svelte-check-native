// Simulated overlay emit for App.svelte — the clean consumer exercising
// every pattern the refactor has to cover. No deliberate errors here;
// this file must type-check with zero diagnostics.
//
// Source Svelte (conceptual):
//   <script lang="ts">
//       import Switch from './Switch.svelte';
//       import Wrapper from './Wrapper.svelte';
//       import VirtualList from './VirtualList.svelte';
//       import { count } from './store';
//
//       type Row = { id: number; label: string };
//
//       let isOn = $state(false);
//       let inputEl: HTMLInputElement | null = $state(null);
//       let nameValue: string = $state('');
//       let items: Row[] = $state([{ id: 1, label: 'a' }, { id: 2, label: 'b' }]);
//       let threshold = $state(5);
//   </script>
//
//   <Switch checked={isOn} onchange={({ checked }) => (isOn = checked)} />
//   <input bind:this={inputEl} />
//   <input bind:value={nameValue} />
//   <p>Count: {$count}</p>
//   {#each items as item, i}
//       <p>#{i}: {item.label}</p>
//   {/each}
//   {#if threshold > 10}
//       <span>big</span>
//   {:else}
//       <span>small</span>
//   {/if}
//   <Wrapper items={items}>
//       {#snippet row({ id, label })}
//           <td>{id}: {label}</td>
//       {/snippet}
//   </Wrapper>
//   <VirtualList items={items}>
//       {#snippet children(item)}
//           <span>{item.label}</span>
//       {/snippet}
//   </VirtualList>

import Switch from './Switch.svelte.ts';
import Wrapper from './Wrapper.svelte.ts';
import VirtualList from './VirtualList.svelte.ts';
import { count } from './store';

async function $$render_app() {
    // Forward-declared store value so the template body can read `$count`
    // before the store import is evaluated. Same trick as today's emit.
    let $count!: __SvnStoreValue<typeof count>;

    type Row = { id: number; label: string };

    let isOn = $state(false);
    let inputEl: HTMLInputElement | null = $state(null);
    let nameValue: string = $state('');
    let items: Row[] = $state([
        { id: 1, label: 'a' },
        { id: 2, label: 'b' },
    ]);
    let threshold = $state(5);

    async function __svn_tpl_check() {
        // <Switch checked={isOn} onchange={({ checked }) => (isOn = checked)} />
        // Component-as-callable: `onchange`'s param destructure picks up
        // contextual type `(event: { checked: boolean }) => void` from
        // the call signature's props.onchange slot. `{ checked }` is
        // typed boolean — no implicit-any.
        Switch(__svn_any(), {
            checked: isOn,
            onchange: ({ checked }) => (isOn = checked),
        });

        // <input bind:this={inputEl} />
        // Asserts inputEl's declared type accepts HTMLInputElement | null | undefined.
        __svn_bind_this_check<HTMLInputElement>(inputEl);

        // <input bind:value={nameValue} />
        // Two-way bind pair: the `value` attribute would normally be
        // checked against <input>'s HTMLInputAttributes. We don't model
        // element-attribute types in Phase A; represent the bind pair as
        // a pure local type-assignment pair. (Phase B may add element
        // attrs.)
        let __svn_bind_input_value_0: string = nameValue;
        nameValue = __svn_bind_input_value_0;
        void __svn_bind_input_value_0;

        // <p>{$count}</p>
        void $count;

        // {#each items as item, i} ... {/each}
        for (const item of __svn_each_items(items)) {
            let i: number = 0;
            void i;
            void item.label;
        }

        // {#if threshold > 10} ... {:else} ... {/if}
        if (threshold > 10) {
            // narrowed branch
        } else {
            // narrowed branch
        }

        // <Wrapper items={items}>
        //     {#snippet row({ id, label })} ... {/snippet}
        // </Wrapper>
        // Snippet contextually typed as Snippet<[{id, label}]>, so the
        // arrow's destructure binds id:number, label:string.
        Wrapper(__svn_any(), {
            items: items,
            row: ({ id, label }) => {
                void id;
                void label;
                return __svn_snippet_return();
            },
        });

        // <VirtualList items={items}>
        //     {#snippet children(item)} ... {/snippet}
        // </VirtualList>
        // Generic VirtualList: T inferred from items (= Row), flows into
        // snippet's param type (= Row). `item.label` type-checks.
        VirtualList(__svn_any(), {
            items: items,
            children: (item) => {
                void item.label;
                return __svn_snippet_return();
            },
        });

        // bind:prop round-trip pair for completeness: bind the Switch's
        // `checked` prop to a local. Uses __SvnProps<>.
        Switch(__svn_any(), {
            checked: isOn,
            onchange: () => {},
        });
        let __svn_bind_checked_0!: __SvnProps<typeof Switch>['checked'];
        isOn = __svn_bind_checked_0;
        void __svn_bind_checked_0;
    }
    void __svn_tpl_check;
    void Switch;
    void Wrapper;
    void VirtualList;
    void count;
    void isOn;
    void inputEl;
    void nameValue;
    void items;
    void threshold;
}
$$render_app;

declare const __svn_component_default: any;
export default __svn_component_default;
