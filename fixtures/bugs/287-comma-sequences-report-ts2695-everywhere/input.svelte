<script lang="ts">
    let a = 1, b = 2;
    $: void (a, b);
    $: (a, b);
    function f() { void (a, b); }
    f();
</script>

<p>{a}{b}</p>
