<script lang="ts">
  export let format = (n: number) => n.toFixed(1);
  export let parse = ({ s }: { s: string }) => s.length;
</script>
{format(1)}{parse({ s: '' })}
