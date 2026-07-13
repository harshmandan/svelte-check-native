<script lang="ts">
    // Untyped $props() with a nested destructure element. The
    // synthesised $$ComponentProps must be `{ age: any } &
    // Record<string, any>`: `age` survives as a prop key, the nested
    // `user` element widens the type instead of becoming an invented
    // required prop, and `name` never surfaces as a key.
    let { user: { name }, age } = $props();
    void name;
    void age;
</script>
