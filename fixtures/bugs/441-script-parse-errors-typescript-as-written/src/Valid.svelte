<script lang="ts">
	interface Point {
		x: number;
	}
	interface Point {
		y: number;
	}
	function len(p: Point): number;
	function len(p: Point) {
		return p.x + p.y;
	}
	class Box {
		constructor(public size: number) {}
	}
	console.log(len({ x: 1, y: 2 }), new Box(1));
</script>
