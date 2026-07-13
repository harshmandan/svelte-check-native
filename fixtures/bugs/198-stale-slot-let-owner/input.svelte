<script lang="ts">
    import Bar from './Bar.svelte';
    import Foo from './Foo.svelte';
    import Wrapper from './Wrapper.svelte';
</script>

<!-- `<Foo />` has no let: directives, so it must NOT become the
     let-owner for the LATER let:thing on Bar. Bar carries slot="x"
     (consumer wrapper), so its let:thing is deliberately left
     unresolved — the forwarded `a={thing}` slot attr drops instead
     of being typed from Foo's default slot. -->
<Wrapper>
    <Foo />
    <Bar slot="x" let:thing>
        <slot a={thing} />
    </Bar>
</Wrapper>
