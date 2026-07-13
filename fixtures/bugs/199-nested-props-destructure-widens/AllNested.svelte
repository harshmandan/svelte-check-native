<script lang="ts">
    // Every leaf nested: upstream emits bare `Record<string, any>` —
    // no invented `user` requirement, no `age` prop key.
    let { user: { name, age } } = $props();
    void name;
    void age;
</script>
