<script lang="ts">
import { writable } from "svelte/store";
const count = writable(0);
const $count = 5;
console.log($count, count);
</script>
