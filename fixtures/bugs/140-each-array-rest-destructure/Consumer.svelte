<script lang="ts">
    import Producer from './Producer.svelte'

    function takesNumber(n: number): void {
        void n
    }
    function takesString(s: string): void {
        void s
    }
    function takesBoolean(b: boolean): void {
        void b
    }

    const rows: [number, string, boolean][] = [
        [1, 'a', true],
        [2, 'b', false],
    ]
</script>

<Producer {rows} let:head let:tail>
    {takesNumber(head)}
    {takesString(tail[0])}
    {takesBoolean(tail[1])}
    <!-- Wrong shape: tail[0] is string, not number. Must fire TS2345. -->
    {takesNumber(tail[0])}
</Producer>
