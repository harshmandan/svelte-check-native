<script lang="ts">
    import Producer from './Producer.svelte'

    function takesNumber(n: number): void {
        void n
    }
    function takesString(s: string): void {
        void s
    }

    const rows = [{ id: 1, label: 'one' }]
</script>

<Producer {rows} let:doubled let:wrapped>
    {takesNumber(doubled)}
    {takesString(wrapped.inner)}
    <!-- swap arg types — both must fire TS2345 -->
    {takesString(doubled)}
    {takesNumber(wrapped.inner)}
</Producer>
