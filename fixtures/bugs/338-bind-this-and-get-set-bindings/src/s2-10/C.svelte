<script lang="ts">let h = $state('');</script>
<div contenteditable   bind:innerHTML={() => h, (v: number) => {}}></div>
