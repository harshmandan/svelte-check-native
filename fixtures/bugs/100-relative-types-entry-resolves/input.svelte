<script lang="ts">
    // `WorkerEnv` only exists in worker-configuration.d.ts, which the
    // tsconfig pulls in via a relative `compilerOptions.types` entry.
    const env: WorkerEnv = { SECRET: 'x' };
</script>

<p>{env.SECRET}</p>
