<script lang="ts">let s: string = $state('');</script>
<input type="number" bind:value={() => s, v => s = v}>
