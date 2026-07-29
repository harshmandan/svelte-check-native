<script lang="ts" generics="Row, Col extends { key: string }">
interface Props {
  rows: Row[]
  columns: Col[]
  render: (row: Row, col: Col) => string
}

let { rows, columns, render }: Props = $props()
</script>

{#each rows as row}
  <tr>
    {#each columns as col}
      <td>{render(row, col)}</td>
    {/each}
  </tr>
{/each}
