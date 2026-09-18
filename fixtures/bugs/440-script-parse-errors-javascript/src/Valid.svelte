<script>
	var a = 1;
	var a = 2;
	let b = 1;
	{
		let b = 2;
		console.log(b);
	}
	try {
		console.log(a);
	} catch (e) {
		var e2 = e;
		console.log(e2);
	}
	loop: for (const x of [1, 2]) {
		if (x) continue loop;
	}
	class P {
		#v = 1;
		get #w() {
			return this.#v;
		}
		set #w(n) {
			this.#v = n;
		}
		m() {
			return this.#w;
		}
	}
	console.log(a, b, new P());
</script>
