<script lang="ts">
    import Producer from './Producer.svelte'

    function takesString(s: string): void {
        void s
    }
    function takesStringArray(a: string[]): void {
        void a
    }
    function takesNumberArray(a: number[]): void {
        void a
    }

    const rows: string[][] = [['a', 'b', 'c']]
</script>

<Producer {rows} let:head let:tail>
    {takesString(head)}
    {takesStringArray(tail)}
    <!-- Wrong shape: tail is string[], not number[]. Must fire TS2345. -->
    {takesNumberArray(tail)}
</Producer>
