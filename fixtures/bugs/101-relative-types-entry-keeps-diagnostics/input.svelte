<script lang="ts">
    const env: WorkerEnv = { SECRET: 'x' };
    const named: Array<[string, string[]]> = [['a', [env.SECRET]]];
    const ungrouped: string[] = ['u'];

    function build() {
        const sections = named.map(([name, rules]) => ({ name, rules }));
        // `name` infers as `string`, so pushing `null` is a real error.
        if (ungrouped) sections.push({ name: null, rules: ungrouped });
        return sections;
    }

    const out = build();
</script>

<p>{out.length}</p>
