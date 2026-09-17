<script lang="ts">import type { state } from './types'; let n = $state(0); const m: state = 1;</script>
<button onclick={() => n++}>{n}{m}</button>
