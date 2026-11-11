<script lang="ts" generics="Keys extends { key?: string }">
	type Validate<Key extends string> = Key extends 'enter' | 'escape' ? Key : never

	let { key }: Keys & { key?: Validate<NonNullable<Keys['key']>> } = $props()
</script>

<kbd>{key}</kbd>
