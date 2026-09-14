<script lang="ts">
    type T = $$Generic<{ a: string, b: number }>;
    export let x: T;
</script>

<p>{x.a}</p>
