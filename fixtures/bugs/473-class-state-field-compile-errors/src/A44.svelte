<script lang="ts">
	let x = $state(0) as number;
	let y = ($derived(x));
	class A {
		z: number;
		constructor() {
			(this.z = $state(0));
		}
	}
	console.log(y, A);
</script>
