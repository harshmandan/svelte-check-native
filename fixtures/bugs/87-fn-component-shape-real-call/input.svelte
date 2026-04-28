<script lang="ts">
// Real `createEventDispatcher()` call — must disqualify the
// `__sveltets_2_fn_component` default-export shape and route
// through the iso-component interface so child events surface
// at consumer sites.
import { createEventDispatcher } from 'svelte'

const dispatch = createEventDispatcher<{ click: { id: number } }>()

let { value }: { value: number } = $props()

function handle() {
    dispatch('click', { id: value })
}
</script>

<button onclick={handle}>{value}</button>
