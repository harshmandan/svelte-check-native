<script lang="ts">
export let settings: { goalPrompt: string; n: number }
let goalPrompt: string
let n: number
$: ({
  goalPrompt,
  n,
} = settings)
let originalGoalPrompt = goalPrompt
</script>
{originalGoalPrompt}{n}
