<script lang="ts">export let n = 1;</script>
{#snippet row()}<slot name="r" x={1} />{/snippet}
{#snippet uses()}<slot name="u" {n} />{/snippet}
