<script lang="ts">
    import Producer from './Producer.svelte'

    function takesNumber(n: number): void {
        void n
    }
    function takesString(s: string): void {
        void s
    }

    const fakeRows = {
        length: 1,
        0: { id: 7, vals: [1, 2, 3] },
    }
</script>

<Producer rows={fakeRows} let:doubled>
    <!-- doubled resolves to `number` via __SvnEachItem<MyArrayLike>.id. -->
    {takesNumber(doubled)}
    <!-- Wrong shape: doubled is number, not string. MUST fire TS2345. -->
    {takesString(doubled)}
</Producer>
