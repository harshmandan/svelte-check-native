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

<Producer {rows} let:id let:rest>
    {takesNumber(id)}
    {takesString(rest.label)}
    <!-- rest excludes 'id' — accessing rest.id should fail. -->
    {takesNumber(rest.id)}
</Producer>
