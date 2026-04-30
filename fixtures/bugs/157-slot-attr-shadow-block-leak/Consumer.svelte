<script lang="ts">
    import Producer from './Producer.svelte'

    function takesNumberArray(a: number[]): void {
        void a
    }
</script>

<Producer rows={[{ id: 1, vals: [1, 2, 3] }]} let:doubled>
    {takesNumberArray(doubled)}
</Producer>
