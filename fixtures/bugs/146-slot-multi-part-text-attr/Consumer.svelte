<script lang="ts">
    import Producer from './Producer.svelte'

    function takesString(s: string): void {
        void s
    }
    function takesNumber(n: number): void {
        void n
    }
</script>

<Producer name="x" let:label>
    {takesString(label)}
    <!-- Wrong shape: label is string, not number. Must fire TS2345. -->
    {takesNumber(label)}
</Producer>
