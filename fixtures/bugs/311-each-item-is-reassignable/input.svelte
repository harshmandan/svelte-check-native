<script lang="ts">export let names: string[] = [];</script>
{#each names as name, i}
  <input bind:value={name} />
  <button on:click={() => (name = '')}>clear {i}</button>
{/each}
