<script lang="ts">
    import Producer from './Producer.svelte'

    function takesUnion(v: number | string): void {
        void v
    }
    function takesNumber(n: number): void {
        void n
    }

    const rows: { id: number; label: string }[] = [{ id: 1, label: 'a' }]
    const key: 'id' | 'label' = 'id'
</script>

<Producer {rows} {key} let:value>
    {takesUnion(value)}
    <!-- value is `number | string` (Row[typeof key]). takesNumber
         expects `number`, must fire TS2345. -->
    {takesNumber(value)}
</Producer>
