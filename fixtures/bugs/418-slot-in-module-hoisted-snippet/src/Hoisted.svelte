<script lang="ts" context="module">export const M = 1;</script>
<script lang="ts">let q = 1;</script>
{#snippet row()}<slot name="r" x={M} />{/snippet}
{#snippet inner()}<div><slot /></div>{/snippet}
{q}
