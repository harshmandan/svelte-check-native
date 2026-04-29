<script lang="ts">
    import Producer from './Producer.svelte'

    function takesAny(_x: any): void {}

    const promise: Promise<{ ok: true }> = Promise.resolve({ ok: true })
</script>

<Producer {promise} let:name let:message>
    {takesAny(name)}
    {takesAny(message)}
</Producer>
