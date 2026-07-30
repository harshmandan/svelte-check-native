<script lang="ts">
interface Props {
  label: string
}

let { label }: Props = $props()
</script>

<span>{label}</span>
