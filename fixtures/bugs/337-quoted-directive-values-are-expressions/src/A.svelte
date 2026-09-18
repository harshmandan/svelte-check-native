<script lang="ts">function handler(n: number) {} let x = 1; let cond = true; function h(n: string) {}
import Child from './Child.svelte';</script>
<button on:click="{handler}">x</button>
<Child on:go="{h}" />
<input type="text" bind:value="{x}">
<div class:a="{cond}"></div>
