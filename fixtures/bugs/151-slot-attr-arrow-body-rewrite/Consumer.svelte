<script lang="ts">
    import Producer from './Producer.svelte'

    function takesNumberArray(a: number[]): void {
        void a
    }
    function takesStringArray(a: string[]): void {
        void a
    }
</script>

<Producer rows={[{ id: 1, vals: [1, 2, 3] }]} let:doubled>
    {takesNumberArray(doubled)}
    <!-- Wrong shape: doubled is number[], not string[]. Must fire TS2345. -->
    {takesStringArray(doubled)}
</Producer>
