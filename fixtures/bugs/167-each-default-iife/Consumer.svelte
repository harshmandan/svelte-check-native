<script lang="ts">
    import Producer from './Producer.svelte'

    function takesString(s: string): void {
        void s
    }
</script>

<Producer rows={[{}]} let:v>
    {takesString(v)}
</Producer>
