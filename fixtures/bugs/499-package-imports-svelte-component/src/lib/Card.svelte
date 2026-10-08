<script lang="ts" module>
  export type Scope = 'org' | 'class';
</script>

<script lang="ts">
  let { scope }: { scope: Scope } = $props();
</script>

<p>{scope}</p>
