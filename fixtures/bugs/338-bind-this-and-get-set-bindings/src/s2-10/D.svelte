<script lang="ts">let h = $state('');</script>
<div bind:innerHTML={() => h, (v: number) => {}} contenteditable></div>
