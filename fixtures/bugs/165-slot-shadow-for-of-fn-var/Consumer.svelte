<script lang="ts">
    import Producer from './Producer.svelte'

    function takesNumber(n: number): void {
        void n
    }
</script>

<Producer rows={[{ id: 1, vals: [1, 2, 3] }]} let:a let:b let:c>
    {takesNumber(a)}
    {takesNumber(b)}
    {takesNumber(c)}
</Producer>
