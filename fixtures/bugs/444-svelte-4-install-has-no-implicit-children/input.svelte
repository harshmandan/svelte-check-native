<script lang="ts">
  import Select from './lib/Select';
  export let search: () => unknown[];
</script>

<Select loadOptions={search}>
  <span>slot content</span>
</Select>
