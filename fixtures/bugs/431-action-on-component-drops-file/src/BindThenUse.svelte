<script lang="ts">import C from "./C.svelte"; let k = 1;</script>
<C bind:value={k()} use:k />
