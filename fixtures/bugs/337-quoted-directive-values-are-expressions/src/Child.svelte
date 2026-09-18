<script lang="ts">import { createEventDispatcher } from 'svelte'; const d = createEventDispatcher<{ go: number }>(); export let v = '';</script>
<button on:click={() => d('go', 1)}>{v}</button>
