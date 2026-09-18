<script lang="ts">let items = $state([1]); function f() { return 1; }</script>
{#each items as item}<li>static</li>{/each}
{#if items}{@const x = f()}<p>y</p>{/if}
{#snippet row(a: number, b: number)}{a}{/snippet}
{@render row(1, 2)}
